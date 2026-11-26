//! Unit tests over synthetic ABFs (see `synth.rs`).

use crate::synth::{self, Options};
use crate::{Abf, Error, FileKind, PAGE_SIZE, SystemTable};

fn sample(opts: &Options) -> Vec<u8> {
    synth::build(&synth::sample_files(), opts)
}

/// Rewrites the header XML (UTF-16LE from offset 72) with `from` replaced by `to`.
fn patch_header(stream: &mut [u8], from: &str, to: &str) {
    let page = stream.get(72..PAGE_SIZE).unwrap();
    let units: Vec<u16> = page
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u16::from_le_bytes(*p))
        .take_while(|&u| u != 0)
        .collect();
    let xml = String::from_utf16(&units).unwrap();
    assert!(xml.contains(from), "{from} not in header");
    let new: Vec<u8> = xml
        .replace(from, to)
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let dst = stream.get_mut(72..PAGE_SIZE).unwrap();
    dst.fill(0);
    dst[..new.len()].copy_from_slice(&new);
}

#[test]
fn parses_power_bi_shaped_stream() {
    let bytes = sample(&Options::default());
    assert_eq!(bytes.len() % PAGE_SIZE, 0);
    let abf = Abf::parse(&bytes).unwrap();
    let files = abf.files();
    assert_eq!(files.len(), synth::sample_files().len() + 3);
    assert_eq!(files[0].kind, FileKind::AdditionalLog);
    assert_eq!(files[1].kind, FileKind::Partitions);
    assert_eq!(files.last().unwrap().kind, FileKind::BackupLog);
    assert_eq!(abf.header().sync_version, Some(140));
    assert!(!abf.header().crc_markers);
    assert_eq!(abf.backup_log().sync_version.as_deref(), Some("11.53"));
    assert_eq!(abf.backup_log().is_tabular, Some(true));

    for (path, data) in synth::sample_files() {
        assert_eq!(abf.read(path), Some(data), "{path}");
        let e = abf.entry(path).unwrap();
        assert_eq!(e.size, data.len());
        assert_eq!(e.stored_size, data.len());
        assert_eq!(e.storage_path.len(), 20);
        assert!(
            e.original_path
                .as_deref()
                .unwrap()
                .starts_with(synth::PERSIST_ROOT)
        );
        assert_eq!(e.file_group_class, Some(100069));
    }
    assert_eq!(abf.metadata_sqlitedb().unwrap(), b"SQLite format 3\0synthetic");
    let by_name = |n: &str| files.iter().find(|f| f.name().ends_with(n)).unwrap();
    assert_eq!(by_name("CryptKey.bin").kind, FileKind::CryptKey);
    assert_eq!(by_name("(14).0.idf").kind, FileKind::Idf);
    assert_eq!(by_name("(14).0.idfmeta").kind, FileKind::IdfMeta);
    assert_eq!(by_name(".dictionary").kind, FileKind::Dictionary);
    assert_eq!(
        by_name(".dictionary").name(),
        "1.Sales (12).Name & Co (15).dictionary"
    );
    assert_eq!(by_name(".hidx").kind, FileKind::HashIndex);
    assert_eq!(
        by_name("POS_TO_ID.0.idf").system_table,
        Some(SystemTable::AttributeHierarchy)
    );
    assert_eq!(
        by_name("INDEX.0.idf").system_table,
        Some(SystemTable::Relationship)
    );
    assert_eq!(by_name("(14).0.idf").system_table, None);
}

#[test]
fn lookups_are_tolerant() {
    let bytes = sample(&Options::default());
    let abf = Abf::parse(&bytes).unwrap();
    assert_eq!(abf.read("METADATA.SQLITEDB").map(<[u8]>::len), Some(25));
    assert_eq!(
        abf.read(r"Sales (12).tbl\13.prt\1.Sales (12).Amount (14).0.idf"),
        Some(&b"idf-amount"[..])
    );
    let storage = abf.entry("0.CryptKey.bin").unwrap().storage_path.clone();
    assert_eq!(abf.read(&storage), Some(&b"\x01\x02\x03"[..]));
    assert_eq!(abf.read("LOG").map(|d| &d[..2]), Some(&[0xFF, 0xFE][..]));
    assert!(abf.read("missing.idf").is_none());
}

#[test]
fn summary_counts_without_names() {
    let bytes = sample(&Options::default());
    let s = Abf::parse(&bytes).unwrap().summary();
    assert_eq!(s.file_count, 12);
    assert_eq!(s.kinds.get(&FileKind::Idf), Some(&4));
    assert_eq!(s.system_tables.get(&SystemTable::AttributeHierarchy), Some(&1));
    assert!(s.has_metadata_sqlitedb);
    assert_eq!(s.unmatched_log_records, 0);
    let text = s.to_string();
    assert!(text.contains("idf=4"), "{text}");
    assert!(!text.contains("Sales"), "summary must not leak names: {text}");
}

#[test]
fn crc_marker_flavor_excludes_markers_from_payload() {
    let bytes = sample(&Options {
        crc_markers: true,
        ..Options::default()
    });
    let abf = Abf::parse(&bytes).unwrap();
    assert!(abf.header().crc_markers);
    let e = abf.entry("Sales (12).tbl/1.Sales (12).Amount (14).hidx").unwrap();
    assert_eq!((e.size, e.stored_size), (4, 8));
    assert_eq!(abf.read(&e.path.clone()), Some(&b"hidx"[..]));
    // The log itself has no log record, so it keeps its marker; the XML reader ignores it.
    assert_eq!(abf.backup_log().file_groups.len(), 1);
}

#[test]
fn utf16_directory_and_no_bookkeeping() {
    let bytes = sample(&Options {
        dir_utf16: true,
        power_bi_bookkeeping: false,
        ..Options::default()
    });
    let abf = Abf::parse(&bytes).unwrap();
    assert_eq!(abf.files().len(), 10);
    assert!(abf.metadata_sqlitedb().is_some());
}

#[test]
fn empty_model_parses() {
    let bytes = synth::build(&[], &Options::default());
    let abf = Abf::parse(&bytes).unwrap();
    assert_eq!(abf.files().len(), 3);
    assert!(abf.metadata_sqlitedb().is_none());
}

#[test]
fn rejects_compressed_and_foreign_input() {
    let banner: Vec<u8> = "This backup was created using XPress9 compression."
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .chain([0u8; 64])
        .collect();
    assert_eq!(Abf::parse(&banner).unwrap_err(), Error::Xpress9Compressed);
    assert!(matches!(Abf::parse(b""), Err(Error::NotAbf(_))));
    assert!(matches!(Abf::parse(b"PK\x03\x04"), Err(Error::NotAbf(_))));
    let mut no_sig = sample(&Options::default());
    no_sig[10] = b'X';
    assert!(matches!(Abf::parse(&no_sig), Err(Error::NotAbf(_))));
    let mut no_bom = sample(&Options::default());
    no_bom[0] = 0;
    assert!(matches!(Abf::parse(&no_bom), Err(Error::NotAbf(_))));
}

#[test]
fn rejects_encrypted_and_reports_compressed() {
    let mut enc = sample(&Options::default());
    patch_header(&mut enc, "<EncryptionFlag>false", "<EncryptionFlag>true");
    assert!(matches!(Abf::parse(&enc), Err(Error::Unsupported(_))));

    // ApplyCompression with an unreadable directory is reported as unsupported, not malformed.
    let mut comp = sample(&Options::default());
    patch_header(&mut comp, "<ApplyCompression>false", "<ApplyCompression>true");
    let dir = Abf::parse(&sample(&Options::default()))
        .unwrap()
        .header()
        .directory_offset as usize;
    comp[dir] = 0x07;
    assert!(matches!(Abf::parse(&comp), Err(Error::Unsupported(_))));
}

/// `ApplyCompression` (Excel's per-file compression): the files the BackupLog describes are
/// marked compressed, `data()` refuses them and `stored()` gives their stored bytes; the log
/// itself is read as stored.
#[test]
fn apply_compression_marks_logged_files_compressed() {
    let mut comp = sample(&Options::default());
    patch_header(&mut comp, "<ApplyCompression>false", "<ApplyCompression>true");
    let abf = Abf::parse(&comp).unwrap();
    for (path, data) in synth::sample_files() {
        let e = abf.entry(path).unwrap();
        assert!(e.compressed, "{path}");
        assert_eq!(abf.data(e), None, "{path}");
        assert_eq!(abf.stored(e), Some(data), "{path}");
    }
    let log = abf.files().last().unwrap();
    assert!(!log.compressed);
    assert!(abf.data(log).is_some());
    // without the flag nothing is compressed
    let plain = sample(&Options::default());
    let abf = Abf::parse(&plain).unwrap();
    assert!(abf.files().iter().all(|e| !e.compressed && abf.data(e).is_some()));
}

#[test]
fn rejects_out_of_bounds_directory() {
    let mut bytes = sample(&Options::default());
    let abf_len = bytes.len();
    patch_header(&mut bytes, "<DataSize>", &format!("<DataSize>{abf_len}"));
    assert!(matches!(Abf::parse(&bytes), Err(Error::OutOfBounds { .. })));
    let mut huge = sample(&Options::default());
    patch_header(
        &mut huge,
        "<m_cbOffsetHeader>",
        "<m_cbOffsetHeader>99999999999999999",
    );
    assert!(matches!(
        Abf::parse(&huge),
        Err(Error::OutOfBounds { .. }) | Err(Error::Malformed { .. })
    ));
    let mut missing = sample(&Options::default());
    patch_header(&mut missing, "m_cbOffsetHeader>", "m_cbOffsetHeaderX>");
    assert!(matches!(Abf::parse(&missing), Err(Error::Malformed { .. })));
}

/// Replaces `from` with `to` (same length) once in the raw stream.
fn patch_raw(bytes: &mut [u8], from: &[u8], to: &[u8]) {
    assert_eq!(from.len(), to.len());
    let pos = bytes.windows(from.len()).position(|w| w == from).unwrap();
    bytes[pos..pos + to.len()].copy_from_slice(to);
}

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// Rewrites the (UTF-8) virtual directory with `from` replaced by `to`, fixing `DataSize`.
fn patch_dir(stream: &[u8], from: &str, to: &str) -> Vec<u8> {
    let h = Abf::parse(stream).unwrap().header().clone();
    let (off, size) = (h.directory_offset as usize, h.directory_size as usize);
    let xml = std::str::from_utf8(&stream[off..off + size]).unwrap();
    assert!(xml.contains(from), "{from} not in directory");
    let new = xml.replacen(from, to, 1);
    let mut out = stream[..off].to_vec();
    out.extend_from_slice(new.as_bytes());
    out.resize(out.len().div_ceil(PAGE_SIZE) * PAGE_SIZE, 0);
    patch_header(
        &mut out,
        &format!("<DataSize>{size}<"),
        &format!("<DataSize>{}<", new.len()),
    );
    out
}

#[test]
fn rejects_inconsistent_directory_and_log() {
    let base = sample(&Options::default());
    assert!(Abf::parse(&patch_dir(&base, "<Path>PARTITIONS</Path>", "<Path>P2</Path>")).is_ok());

    // Directory entry pointing past the end of the stream.
    let oob = patch_dir(
        &base,
        "<m_cbOffsetHeader>4096<",
        "<m_cbOffsetHeader>18446744073709551615<",
    );
    assert!(matches!(Abf::parse(&oob), Err(Error::OutOfBounds { .. })));
    let oob = patch_dir(&base, "<Size>", "<Size>1000000");
    assert!(matches!(Abf::parse(&oob), Err(Error::OutOfBounds { .. })));
    let nan = patch_dir(&base, "<Size>", "<Size>x");
    assert!(matches!(Abf::parse(&nan), Err(Error::Malformed { .. })));
    let no_size = patch_dir(&base, "<Size>", "<Sise>").to_vec();
    assert!(Abf::parse(&no_size).is_err());
    let wrong_root = patch_dir(&base, "<VirtualDirectory>", "<Other>");
    assert!(matches!(Abf::parse(&wrong_root), Err(Error::Malformed { .. })));

    // Duplicate storage names in the directory (case-insensitive).
    let dup = patch_dir(&base, "<Path>ADDITIONAL_LOG</Path>", "<Path>partitions</Path>");
    assert!(matches!(Abf::parse(&dup), Err(Error::Malformed { .. })));

    // Log size larger than the stored size.
    let mut big = base.clone();
    patch_raw(&mut big, &utf16("<Size>4</Size>"), &utf16("<Size>9</Size>"));
    assert!(matches!(Abf::parse(&big), Err(Error::Malformed { .. })));

    // Log records whose storage path is missing are skipped and counted.
    let mut unmatched = base.clone();
    for _ in 0..synth::sample_files().len() {
        patch_raw(
            &mut unmatched,
            &utf16("<StoragePath>00"),
            &utf16("<StoragePath>ZZ"),
        );
    }
    let abf = Abf::parse(&unmatched).unwrap();
    assert_eq!(abf.summary().unmatched_log_records, synth::sample_files().len());
    assert!(abf.metadata_sqlitedb().is_none());
    assert!(abf.files().iter().all(|f| f.original_path.is_none()));
}

#[test]
fn every_truncation_is_handled() {
    let bytes = sample(&Options {
        crc_markers: true,
        ..Options::default()
    });
    let header = Abf::parse(&bytes).unwrap().header().clone();
    let dir_end = (header.directory_offset + header.directory_size) as usize;
    for len in 0..bytes.len() {
        let r = Abf::parse(&bytes[..len]);
        if len < dir_end {
            assert!(r.is_err(), "truncated to {len} parsed");
        } else {
            assert!(r.is_ok(), "zero padding is optional ({len})");
        }
    }
}

#[test]
fn every_byte_flip_is_handled() {
    // Every meaningful byte of the Power BI flavor and every third one of the Excel-like
    // flavor (zero padding is sampled), each with a different mask.
    let flavors = [
        (Options::default(), 1, 0xFFu8),
        (
            Options {
                crc_markers: true,
                dir_utf16: true,
                ..Options::default()
            },
            3,
            0x01,
        ),
    ];
    for (opts, stride, mask) in flavors {
        let bytes = sample(&opts);
        let mut corrupt = bytes.clone();
        for i in (0..bytes.len()).step_by(stride) {
            if bytes[i] == 0 && i % 61 != 0 {
                continue;
            }
            {
                corrupt[i] ^= mask;
                if let Ok(abf) = Abf::parse(&corrupt) {
                    for f in abf.files() {
                        assert!(abf.data(f).is_some());
                    }
                    let _ = abf.summary().to_string();
                    let _ = abf.metadata_sqlitedb();
                }
                corrupt[i] = bytes[i];
            }
        }
    }
}

#[test]
fn random_multi_byte_corruption_is_handled() {
    let bytes = sample(&Options::default());
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..3000 {
        let mut corrupt = bytes.clone();
        for _ in 0..1 + next() % 8 {
            let i = (next() % corrupt.len() as u64) as usize;
            corrupt[i] = (next() & 0xFF) as u8;
        }
        if next() % 4 == 0 {
            let cut = (next() % corrupt.len() as u64) as usize;
            corrupt.truncate(cut);
        }
        if let Ok(abf) = Abf::parse(&corrupt) {
            let _ = abf.summary();
            for f in abf.files() {
                assert!(abf.data(f).is_some());
            }
        }
    }
}
