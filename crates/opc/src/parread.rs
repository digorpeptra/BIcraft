//! Reading a large stored part from a package file on several threads (`docs/storage-engine.md`,
//! "Opening a large PBIX"). A PBIX `DataModel` is stored, not deflated: its bytes are a plain
//! range of the file, read by positional reads into one buffer, each thread taking its own piece
//! and computing the CRC-32 of that piece; the pieces' CRCs are combined and checked against the
//! entry's, as the sequential ZIP reader does.

use std::io;
use std::path::Path;

/// Smallest stored part read on several threads.
pub(crate) const PARALLEL_MIN: u64 = 8 << 20;
/// Most threads one read uses.
const MAX_THREADS: usize = 8;
/// Smallest piece per thread.
const MIN_PIECE: usize = 2 << 20;

/// A zeroed buffer of `n` bytes, or `None` when that much memory cannot be had. The size is
/// first reserved fallibly (and released); the buffer itself then comes from the allocator's
/// zeroed allocation, which takes fresh pages from the system without writing them, so the
/// threads that fill it fault its pages in parallel instead of one thread zeroing it first.
pub(crate) fn zeroed(n: usize) -> Option<Vec<u8>> {
    let mut probe: Vec<u8> = Vec::new();
    probe.try_reserve_exact(n).ok()?;
    drop(probe);
    Some(vec![0u8; n])
}

fn threads_for(len: usize) -> usize {
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    (len / MIN_PIECE).clamp(1, MAX_THREADS.min(cores.max(1)))
}

/// Reads exactly `buf.len()` bytes of `path` at `offset`, on several threads; returns the CRC-32
/// of the bytes read.
pub(crate) fn read_at(path: &Path, offset: u64, buf: &mut [u8]) -> io::Result<u32> {
    let threads = threads_for(buf.len());
    let piece = buf.len().div_ceil(threads).max(1);
    if threads <= 1 {
        let file = std::fs::File::open(path)?;
        return read_piece(&file, offset, buf);
    }
    let results: Vec<io::Result<(crc32fast::Hasher, u64)>> = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        let mut at = offset;
        for chunk in buf.chunks_mut(piece) {
            let start = at;
            let len = u64::try_from(chunk.len()).unwrap_or(u64::MAX);
            at = at.saturating_add(len);
            let job = move || -> io::Result<(crc32fast::Hasher, u64)> {
                let file = std::fs::File::open(path)?;
                fill(&file, start, chunk)?;
                let mut h = crc32fast::Hasher::new();
                h.update(chunk);
                Ok((h, len))
            };
            handles.push(std::thread::Builder::new().spawn_scoped(scope, job));
        }
        handles
            .into_iter()
            .map(|h| match h {
                Ok(h) => h
                    .join()
                    .unwrap_or_else(|_| Err(io::Error::other("a reader thread failed"))),
                Err(e) => Err(e),
            })
            .collect()
    });
    let mut crc = crc32fast::Hasher::new();
    for r in results {
        let (h, _) = r?;
        crc.combine(&h);
    }
    Ok(crc.finalize())
}

fn read_piece(file: &std::fs::File, offset: u64, buf: &mut [u8]) -> io::Result<u32> {
    fill(file, offset, buf)?;
    let mut h = crc32fast::Hasher::new();
    h.update(buf);
    Ok(h.finalize())
}

/// Positional read of exactly `buf.len()` bytes at `offset` (safe `std` positional reads: the
/// file's cursor is not used, so several threads may read one file at once).
#[cfg(unix)]
pub fn fill(file: &std::fs::File, offset: u64, buf: &mut [u8]) -> io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, buf, offset)
}

/// Positional read of exactly `buf.len()` bytes at `offset`.
#[cfg(windows)]
pub fn fill(file: &std::fs::File, offset: u64, buf: &mut [u8]) -> io::Result<()> {
    let mut done = 0usize;
    while done < buf.len() {
        let rest = buf.get_mut(done..).unwrap_or_default();
        let at = offset.saturating_add(u64::try_from(done).unwrap_or(u64::MAX));
        match std::os::windows::fs::FileExt::seek_read(file, rest, at) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
            Ok(n) => done = done.saturating_add(n),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Read of exactly `buf.len()` bytes at `offset`.
#[cfg(not(any(unix, windows)))]
pub fn fill(file: &std::fs::File, offset: u64, buf: &mut [u8]) -> io::Result<()> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = file;
    f.seek(SeekFrom::Start(offset))?;
    f.read_exact(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_range_on_several_threads_with_its_crc() {
        let dir = std::env::temp_dir().join(format!("bicraft-opc-par-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.bin");
        let data: Vec<u8> = (0..(40usize << 20)).map(|i| (i * 31 % 251) as u8).collect();
        std::fs::write(&path, &data).unwrap();
        let (offset, len) = (12_345usize, (40usize << 20) - 20_000);
        let mut buf = zeroed(len).unwrap();
        let crc = read_at(&path, offset as u64, &mut buf).unwrap();
        assert!(threads_for(len) > 1 || std::thread::available_parallelism().map_or(1, |n| n.get()) == 1);
        assert_eq!(buf, data[offset..offset + len]);
        assert_eq!(crc, crc32fast::hash(&data[offset..offset + len]));
        // past the end of the file: an error, never a short buffer
        let mut long = zeroed(len + 20_000).unwrap();
        assert!(read_at(&path, offset as u64, &mut long).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A large stored part of a package file is read on several threads, byte for byte, and a
    /// corrupted byte in it fails the CRC check instead of being returned.
    #[test]
    fn a_large_stored_part_is_read_and_checked() {
        let dir = std::env::temp_dir().join(format!("bicraft-opc-par-pkg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.pbix");
        let big: Vec<u8> = (0..(PARALLEL_MIN as usize + 3_000_001))
            .map(|i| (i * 7 % 253) as u8)
            .collect();
        let mut pkg = crate::Package::default();
        pkg.put("Version", b"1.28".to_vec(), true);
        pkg.put("DataModel", big.clone(), true);
        pkg.put("Report/Layout", vec![7u8; 1000], false);
        let bytes = pkg.to_bytes().unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let back = crate::Package::open(&path).unwrap();
        assert_eq!(back, pkg);
        assert_eq!(back.get("DataModel").unwrap().data, big);
        // one byte flipped in the middle of the part
        let at = bytes
            .windows(64)
            .position(|w| w == &big[1_000_000..1_000_064])
            .unwrap()
            + 5_000_000;
        let mut bad = bytes.clone();
        bad[at] ^= 0x55;
        std::fs::write(&path, &bad).unwrap();
        let err = crate::Package::open(&path).unwrap_err().to_string();
        assert!(err.contains("CRC"), "{err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
