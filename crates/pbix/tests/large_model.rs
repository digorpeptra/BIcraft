//! Reading the model of a `DataModel` too large for memory (docs/formats/pbix-package.md §11,
//! "Packages over 4 GiB"): streamed decoding, deferred parts, and a backup over 4 GiB held
//! sparsely.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;

use bicraft_opc::{ArchiveSource, Deferred, Package, Part, ReadLimits, ReadSeek};
use bicraft_pbix::{
    DATA_MODEL, DataModelLimits, read_data_model, read_data_model_bytes_streamed, read_data_model_streamed,
    read_data_model_with, read_model, synthetic, write_model,
};

fn metadata() -> Vec<u8> {
    synthetic::storage_database().unwrap()
}

#[test]
fn streamed_reads_match_the_in_memory_reader() {
    let pkg = synthetic::pbix(&metadata()).unwrap();
    let part = pkg.get(DATA_MODEL).unwrap();
    let expected = read_data_model(&part.data).unwrap();
    assert_eq!(read_data_model_bytes_streamed(&part.data).unwrap(), expected);
    assert_eq!(read_data_model_streamed(part).unwrap(), expected);
    // A bare ABF part takes the seeking path.
    let abf = synthetic::abf(&[("metadata.sqlitedb", &metadata())]).unwrap();
    assert_eq!(read_data_model_bytes_streamed(&abf).unwrap(), expected);
    // A backup declaring more than `max_bytes` (within the ratio) is streamed, not refused.
    let small = DataModelLimits {
        max_bytes: 1024,
        max_ratio: 1_000_000,
        min_allowance: 0,
    };
    assert_eq!(read_data_model_with(&part.data, &small).unwrap(), expected);
}

#[test]
fn a_deferred_data_model_is_read_streamed_and_kept_on_save() {
    let pkg = synthetic::pbix(&metadata()).unwrap();
    let expected = read_model(&pkg).unwrap();
    let dir = std::env::temp_dir().join(format!("bicraft-pbix-large-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("big.pbix");
    std::fs::write(&path, pkg.to_bytes().unwrap()).unwrap();
    let limits = ReadLimits {
        max_part_bytes: 1024,
        max_total_bytes: 1 << 30,
    };
    let open = Package::open_with(&path, &limits).unwrap();
    let part = open.get(DATA_MODEL).unwrap();
    assert!(part.is_deferred());
    assert_eq!(read_model(&open).unwrap(), expected);
    let inv = bicraft_pbix::inventory(&open);
    let row = inv.parts.iter().find(|p| p.name == DATA_MODEL).unwrap();
    assert!(row.deferred);
    assert_eq!(row.bytes, pkg.get(DATA_MODEL).unwrap().data.len());
    // a deferred part within the 4 GiB part cap is read for a model write (`docs/beyond-ram.md`)
    let written = write_model(&open, &expected).unwrap();
    assert!(!written.get(DATA_MODEL).unwrap().is_deferred());
    assert_eq!(read_model(&written).unwrap(), expected);
    // Report saves copy the part through.
    let back = Package::read_bytes(&open.to_bytes().unwrap()).unwrap();
    assert_eq!(back.get(DATA_MODEL), pkg.get(DATA_MODEL));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// head + zeros + tail, read back as one stream.
#[derive(Debug)]
struct SparseSource(Arc<synthetic::SparseAbf>);

struct SparseReader {
    abf: Arc<synthetic::SparseAbf>,
    pos: u64,
}

impl Read for SparseReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let (h, z) = (self.abf.head.len() as u64, self.abf.zeros);
        let left = self.abf.len().saturating_sub(self.pos);
        let n = (buf.len() as u64).min(left).min(1 << 20) as usize;
        for (i, b) in buf[..n].iter_mut().enumerate() {
            let at = self.pos + i as u64;
            *b = if at < h {
                self.abf.head[at as usize]
            } else if at < h + z {
                0
            } else {
                self.abf.tail[(at - h - z) as usize]
            };
        }
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for SparseReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.pos = match to {
            SeekFrom::Start(n) => n,
            SeekFrom::End(d) => self.abf.len().checked_add_signed(d).unwrap(),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d).unwrap(),
        };
        Ok(self.pos)
    }
}

impl ArchiveSource for SparseSource {
    fn open(&self) -> io::Result<Box<dyn ReadSeek>> {
        Ok(Box::new(SparseReader {
            abf: Arc::clone(&self.0),
            pos: 0,
        }))
    }
}

/// A bare 5 GiB backup (a stored `DataModel` left in its file): `metadata.sqlitedb` first, a
/// 5 GiB column file, then the BackupLog and the virtual directory past 4 GiB. The model is
/// read by seeking to the four ranges it needs; nothing near 5 GiB is read or allocated.
#[test]
fn the_model_of_a_backup_over_4_gib_is_read() {
    let meta = metadata();
    let sparse = synthetic::sparse_abf(
        &[("metadata.sqlitedb", &meta)],
        Some(("Sales (3).tbl/4.prt/1.Sales (3).Amount (5).0.idf", 5 << 30)),
    )
    .unwrap();
    let len = sparse.len();
    assert!(len > 5 << 30);
    let part = Part {
        name: DATA_MODEL.into(),
        data: vec![],
        stored: true,
        deferred: Some(Deferred {
            source: Arc::new(SparseSource(Arc::new(sparse))),
            data_offset: 0,
            compressed_size: len,
            size: len,
            crc32: 0,
        }),
    };
    let expected = read_data_model(&synthetic::abf(&[("metadata.sqlitedb", &meta)]).unwrap()).unwrap();
    assert_eq!(read_data_model_streamed(&part).unwrap(), expected);
    let mut pkg = Package::default();
    pkg.parts.push(part);
    assert_eq!(read_model(&pkg).unwrap(), expected);
}
