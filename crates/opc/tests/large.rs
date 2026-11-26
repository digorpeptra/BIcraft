//! Packages over 4 GiB without 4 GiB of memory (docs/formats/pbix-package.md §11, "Packages
//! over 4 GiB"): archives are held sparsely (runs of zeros are not stored), so a 5 GiB part
//! costs only the time to stream it.

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};

use bicraft_opc::{ArchiveSource, Deferred, Package, Part, ReadLimits, ReadSeek};

const GIB: u64 = 1 << 30;

/// An archive kept as stored segments; bytes between segments are zeros.
#[derive(Debug, Default, Clone)]
struct Sparse {
    segments: Vec<(u64, Vec<u8>)>,
    len: u64,
}

impl Sparse {
    fn byte_range(&self, at: u64, buf: &mut [u8]) -> usize {
        let left = self.len.saturating_sub(at);
        let n = (buf.len() as u64).min(left) as usize;
        let out = &mut buf[..n];
        out.fill(0);
        for (start, data) in &self.segments {
            let end = start + data.len() as u64;
            let (lo, hi) = (at.max(*start), (at + n as u64).min(end));
            if lo < hi {
                let src = &data[(lo - start) as usize..(hi - start) as usize];
                out[(lo - at) as usize..(hi - at) as usize].copy_from_slice(src);
            }
        }
        n
    }
}

/// A writer that drops all-zero pieces of 64 KiB or more (they read back as zeros).
struct SparseSink(Sparse);

impl Write for SparseSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        const ZERO: [u8; 4096] = [0; 4096];
        let zeros = buf.len() >= 1 << 16 && buf.chunks(4096).all(|c| c == &ZERO[..c.len()]);
        if !zeros {
            self.0.segments.push((self.0.len, buf.to_vec()));
        }
        self.0.len += buf.len() as u64;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct SparseReader {
    archive: Arc<Sparse>,
    pos: u64,
}

impl Read for SparseReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.archive.byte_range(self.pos, buf);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for SparseReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.pos = match to {
            SeekFrom::Start(n) => n,
            SeekFrom::End(d) => self.archive.len.checked_add_signed(d).unwrap(),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d).unwrap(),
        };
        Ok(self.pos)
    }
}

#[derive(Debug)]
struct SparseSource {
    archive: Arc<Sparse>,
    opened: Mutex<usize>,
}

impl SparseSource {
    fn new(archive: Sparse) -> Arc<SparseSource> {
        Arc::new(SparseSource {
            archive: Arc::new(archive),
            opened: Mutex::new(0),
        })
    }
}

impl ArchiveSource for SparseSource {
    fn open(&self) -> io::Result<Box<dyn ReadSeek>> {
        *self.opened.lock().unwrap() += 1;
        Ok(Box::new(SparseReader {
            archive: Arc::clone(&self.archive),
            pos: 0,
        }))
    }
}

/// CRC-32 of 5 GiB of zeros (`zeros_crc(5 * GIB)`, precomputed: the test streams the part
/// twice already).
const CRC_5_GIB_ZEROS: u32 = 0x1938_38c3;

/// CRC-32 of `n` zero bytes.
fn zeros_crc(n: u64) -> u32 {
    let buf = vec![0u8; 1 << 20];
    let mut h = crc32fast::Hasher::new();
    let mut left = n;
    while left > 0 {
        let k = left.min(buf.len() as u64) as usize;
        h.update(&buf[..k]);
        left -= k as u64;
    }
    h.finalize()
}

fn u16_at(b: &Sparse, at: u64) -> u16 {
    let mut x = [0u8; 2];
    b.byte_range(at, &mut x);
    u16::from_le_bytes(x)
}

fn u32_at(b: &Sparse, at: u64) -> u32 {
    let mut x = [0u8; 4];
    b.byte_range(at, &mut x);
    u32::from_le_bytes(x)
}

fn u64_at(b: &Sparse, at: u64) -> u64 {
    let mut x = [0u8; 8];
    b.byte_range(at, &mut x);
    u64::from_le_bytes(x)
}

/// A 5 GiB stored `DataModel` of zeros (the bytes of a source that is itself all zeros),
/// between two small parts. Written, read back (the big part deferred, the others in memory),
/// written again from the deferred part, and read back once more: same parts, Zip64 where the
/// values need it, and nothing near the part's size allocated.
#[test]
fn a_5_gib_part_round_trips_through_zip64_without_being_held_in_memory() {
    let size = 5 * GIB;
    let crc = CRC_5_GIB_ZEROS;
    let zeros = SparseSource::new(Sparse {
        segments: vec![],
        len: size,
    });
    let mut pkg = Package::default();
    pkg.put("Version", b"1.28".to_vec(), true);
    pkg.parts.push(Part {
        name: "DataModel".into(),
        data: vec![],
        stored: true,
        deferred: Some(Deferred {
            source: zeros,
            data_offset: 0,
            compressed_size: size,
            size,
            crc32: crc,
        }),
    });
    pkg.put("Report/Layout", vec![1, 2, 3], false);

    let first = pkg
        .write_with(SparseSink(Sparse::default()), &Default::default())
        .unwrap()
        .0;
    assert!(first.len > size);
    let src = SparseSource::new(first.clone());
    let back = Package::read_from(src.clone(), &ReadLimits::default()).unwrap();
    assert_eq!(back, pkg);
    assert!(back.has_deferred());
    let big = back.get("DataModel").unwrap();
    assert!(big.data.is_empty());
    assert_eq!(big.len(), size);
    assert_eq!(back.get("Report/Layout").unwrap().data, vec![1, 2, 3]);

    // The entry after the big part sits past 4 GiB: its central record carries the offset in
    // a Zip64 field, and the big entry carries both sizes.
    let eocd = first.len - 22;
    assert_eq!(u32_at(&first, eocd), 0x0605_4b50);
    let cd = u32_at(&first, eocd + 16);
    assert_eq!(cd, u32::MAX, "central directory past 4 GiB");
    let rec = u64_at(&first, eocd - 20 + 8);
    assert_eq!(u32_at(&first, rec), 0x0606_4b50);
    let mut at = u64_at(&first, rec + 48);
    let mut seen = Vec::new();
    for _ in 0..3 {
        assert_eq!(u32_at(&first, at), 0x0201_4b50);
        let (name_len, extra_len) = (u16_at(&first, at + 28) as u64, u16_at(&first, at + 30) as u64);
        seen.push((u32_at(&first, at + 24), u32_at(&first, at + 42), extra_len));
        at += 46 + name_len + extra_len;
    }
    assert_eq!(seen[0], (4, 0, 0), "Version: no Zip64");
    assert_eq!((seen[1].0, seen[1].2), (u32::MAX, 20), "DataModel: both sizes");
    assert_eq!((seen[2].1, seen[2].2), (u32::MAX, 12), "Layout: offset only");

    // Written again from the deferred part: the same archive, byte for byte.
    let second = back
        .write_with(SparseSink(Sparse::default()), &Default::default())
        .unwrap()
        .0;
    assert_eq!(second.len, first.len);
    let mut a = vec![0u8; 1 << 16];
    let mut b = vec![0u8; 1 << 16];
    for at in [0, first.len - (1 << 16)] {
        first.byte_range(at, &mut a);
        second.byte_range(at, &mut b);
        assert_eq!(a, b);
    }
    let again = Package::read_from(SparseSource::new(second), &ReadLimits::default()).unwrap();
    assert_eq!(again, pkg);

    // The decompressed stream of the deferred part.
    assert_eq!(zeros_crc(3 << 20), crc32fast::hash(&vec![0; 3 << 20]));
    let mut r = big.reader().unwrap();
    let mut head = [1u8; 16];
    r.read_exact(&mut head).unwrap();
    assert_eq!(head, [0; 16]);
    let mut s = big.seekable().unwrap().unwrap();
    s.seek(SeekFrom::Start(size - 4)).unwrap();
    let mut tail = Vec::new();
    s.read_to_end(&mut tail).unwrap();
    assert_eq!(tail, [0; 4]);
}

/// A deferred deflated part is copied without being decompressed, and streams back.
#[test]
fn deferred_deflated_parts_are_copied_raw() {
    let mut pkg = Package::default();
    pkg.put("a", vec![7; 3 << 20], false);
    pkg.put("b", b"small".to_vec(), false);
    let bytes = pkg.to_bytes().unwrap();
    let limits = ReadLimits {
        max_part_bytes: 1 << 20,
        max_total_bytes: 1 << 30,
    };
    let mut sink = Sparse::default();
    sink.segments.push((0, bytes.clone()));
    sink.len = bytes.len() as u64;
    let src = SparseSource::new(sink);
    let back = Package::read_from(src.clone(), &limits).unwrap();
    assert!(back.get("a").unwrap().is_deferred());
    assert!(!back.get("b").unwrap().is_deferred());
    assert_eq!(back.to_bytes().unwrap(), bytes, "raw copy, same bytes");
    let mut all = Vec::new();
    back.get("a")
        .unwrap()
        .reader()
        .unwrap()
        .read_to_end(&mut all)
        .unwrap();
    assert_eq!(all, vec![7; 3 << 20]);
    assert!(back.get("a").unwrap().seekable().unwrap().is_none());
    // Without a source to reopen, an oversized part is still refused.
    assert!(Package::read_bytes_with(&bytes, &limits).is_err());
}

/// A source whose bytes changed under a deferred part makes the write fail instead of
/// producing an archive with a wrong CRC.
#[test]
fn a_changed_source_fails_the_write() {
    let mut pkg = Package::default();
    pkg.put("big", vec![9; 2 << 20], true);
    let bytes = pkg.to_bytes().unwrap();
    let limits = ReadLimits {
        max_part_bytes: 1 << 20,
        max_total_bytes: 1 << 30,
    };
    let src = SparseSource::new(Sparse {
        segments: vec![(0, bytes.clone())],
        len: bytes.len() as u64,
    });
    let mut back = Package::read_from(src, &limits).unwrap();
    let mut tampered = bytes.clone();
    let mid = tampered.len() / 2;
    tampered[mid] ^= 1;
    let d = back.parts[0].deferred.as_mut().unwrap();
    d.source = SparseSource::new(Sparse {
        segments: vec![(0, tampered)],
        len: bytes.len() as u64,
    });
    let err = back.to_bytes().unwrap_err().to_string();
    assert!(err.contains("changed"), "{err}");
}

/// After a save to a new archive, the deferred parts point into it.
#[test]
fn deferred_parts_rebind_to_the_written_archive() {
    let dir = std::env::temp_dir().join(format!("bicraft-opc-large-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("a.pbix");
    let mut pkg = Package::default();
    pkg.put("Version", b"1".to_vec(), true);
    pkg.put("big", vec![5; 2 << 20], true);
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    let limits = ReadLimits {
        max_part_bytes: 1 << 20,
        max_total_bytes: 1 << 30,
    };
    let mut open = Package::open_with(&path, &limits).unwrap();
    assert!(open.get("big").unwrap().is_deferred());
    // Save in place with a new small part in front: temporary sibling, then rename.
    open.parts.insert(0, Part::new("Metadata", vec![1; 100], false));
    let tmp = dir.join("a.pbix.tmp");
    open.write_with(std::fs::File::create(&tmp).unwrap(), &Default::default())
        .unwrap();
    std::fs::rename(&tmp, &path).unwrap();
    // The old source is gone: writing again fails until the parts are rebound.
    assert!(open.to_bytes().is_err());
    open.rebind_deferred(Arc::new(bicraft_opc::FileSource::new(&path).unwrap()))
        .unwrap();
    let again = Package::read_bytes(&open.to_bytes().unwrap()).unwrap();
    assert_eq!(again.get("big").unwrap().data, vec![5; 2 << 20]);
    std::fs::remove_dir_all(&dir).unwrap();
}
