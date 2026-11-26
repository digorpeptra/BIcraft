//! Writer tests over synthetic ABFs (see `synth.rs`): identity, replacement, hostile input.

use crate::synth::{self, Options};
use crate::writer::{align_page, file_offsets};
use crate::{Abf, Error, FileKind, PAGE_SIZE};

const METADATA: &str = "metadata.sqlitedb";

fn flavors() -> Vec<Options> {
    vec![
        Options::default(),
        Options {
            crc_markers: true,
            ..Options::default()
        },
        Options {
            dir_utf16: true,
            power_bi_bookkeeping: false,
            ..Options::default()
        },
    ]
}

/// Parses `bytes` and checks every structural invariant the writer promises.
fn check_consistent(bytes: &[u8]) -> Abf<'_> {
    let abf = Abf::parse(bytes).unwrap();
    assert_eq!(bytes.len() % PAGE_SIZE, 0);
    let h = abf.header();
    assert_eq!(h.file_count, Some(abf.files().len() as u64));
    assert_eq!(h.data_offset, Some(PAGE_SIZE as u64));
    let mut pos = PAGE_SIZE;
    for f in abf.files() {
        assert_eq!(f.offset, pos, "files are contiguous");
        pos += f.stored_size;
    }
    let dir = h.directory_offset as usize;
    assert_eq!(dir, pos.next_multiple_of(PAGE_SIZE));
    assert!(bytes[pos..dir].iter().all(|&b| b == 0));
    let dir_end = dir + h.directory_size as usize;
    assert_eq!(bytes.len(), dir_end.next_multiple_of(PAGE_SIZE));
    assert!(bytes[dir_end..].iter().all(|&b| b == 0));
    for rec in abf.backup_log().file_groups.iter().flat_map(|g| &g.files) {
        let e = abf.entry(&rec.storage_path).unwrap();
        assert_eq!(rec.size, Some(e.size as u64), "log Size matches the file");
    }
    assert_eq!(abf.summary().unmatched_log_records, 0);
    assert_eq!(abf.files().last().unwrap().kind, FileKind::BackupLog);
    abf
}

#[test]
fn identity_is_byte_exact() {
    for opts in flavors() {
        let input = synth::build(&synth::sample_files(), &opts);
        let abf = Abf::parse(&input).unwrap();
        let out = abf.to_writer().unwrap().finish().unwrap();
        assert!(out == input, "identity round trip differs");
        check_consistent(&out);
    }
    let empty = synth::build(&[], &Options::default());
    let out = Abf::parse(&empty).unwrap().to_writer().unwrap().finish().unwrap();
    assert!(out == empty);
}

#[test]
fn same_size_replacement_touches_only_that_file() {
    let input = synth::build(&synth::sample_files(), &Options::default());
    let abf = Abf::parse(&input).unwrap();
    let target = abf.metadata_entry().unwrap().clone();
    let mut new = abf.metadata_sqlitedb().unwrap().to_vec();
    new.reverse();
    let mut w = abf.to_writer().unwrap();
    w.replace_metadata_sqlitedb(new.clone()).unwrap();
    let out = w.finish().unwrap();
    assert_eq!(out.len(), input.len());
    for (i, (a, b)) in input.iter().zip(&out).enumerate() {
        if a != b {
            assert!(
                (target.offset..target.offset + target.size).contains(&i),
                "byte {i} changed"
            );
        }
    }
    let back = check_consistent(&out);
    assert_eq!(back.metadata_sqlitedb(), Some(&new[..]));
}

#[test]
fn different_size_replacements_round_trip() {
    let files = synth::sample_files();
    let input = synth::build(&files, &Options::default());
    let abf = Abf::parse(&input).unwrap();
    let idf = "Sales (12).tbl/13.prt/1.Sales (12).Amount (14).0.idf";
    // Grow past a page boundary, shrink to empty, and a bookkeeping file.
    let cases: Vec<Vec<(&str, Vec<u8>)>> = vec![
        vec![(METADATA, vec![0x5A; 3 * PAGE_SIZE + 17])],
        vec![(METADATA, Vec::new())],
        vec![(METADATA, b"x".to_vec()), (idf, vec![7; 10_000])],
        vec![("ADDITIONAL_LOG", b"\xFF\xFE<\0a\0/\0>\0".to_vec())],
    ];
    for case in cases {
        let mut w = abf.to_writer().unwrap();
        for (path, data) in &case {
            w.replace(path, data.clone()).unwrap();
        }
        let out = w.finish().unwrap();
        let back = check_consistent(&out);
        assert_eq!(back.files().len(), abf.files().len());
        for (old, new) in abf.files().iter().zip(back.files()) {
            assert_eq!(old.path, new.path);
            assert_eq!(old.storage_path, new.storage_path);
            assert_eq!(old.last_write_time, new.last_write_time, "timestamps are kept");
            let replaced = case.iter().find(|(p, _)| *p == old.path);
            match replaced {
                Some((_, data)) => assert_eq!(back.data(new), Some(&data[..])),
                None if old.kind == FileKind::BackupLog => {}
                None => assert_eq!(back.data(new), abf.data(old), "{} unchanged", old.path),
            }
        }
        // The reader's view of the backup log follows the new sizes.
        for (path, data) in &case {
            assert_eq!(back.entry(path).unwrap().size, data.len());
        }
    }
}

#[test]
fn replacing_back_the_original_is_identity() {
    let input = synth::build(&synth::sample_files(), &Options::default());
    let abf = Abf::parse(&input).unwrap();
    let mut w = abf.to_writer().unwrap();
    w.replace(METADATA, vec![1u8; 9999]).unwrap();
    w.replace(METADATA, abf.metadata_sqlitedb().unwrap()).unwrap();
    assert!(w.finish().unwrap() == input);
}

#[test]
fn utf16_directory_is_patched_in_its_encoding() {
    let opts = Options {
        dir_utf16: true,
        power_bi_bookkeeping: false,
        ..Options::default()
    };
    let input = synth::build(&synth::sample_files(), &opts);
    let abf = Abf::parse(&input).unwrap();
    let mut w = abf.to_writer().unwrap();
    w.replace(METADATA, vec![3u8; 5000]).unwrap();
    let out = w.finish().unwrap();
    let back = check_consistent(&out);
    assert_eq!(back.metadata_sqlitedb(), Some(&[3u8; 5000][..]));
    let dir = &out[back.header().directory_offset as usize..];
    assert_eq!(&dir[..4], b"<\0V\0", "directory stays UTF-16LE");
}

#[test]
fn rewriting_the_output_is_stable() {
    let input = synth::build(&synth::sample_files(), &Options::default());
    let abf = Abf::parse(&input).unwrap();
    let mut w = abf.to_writer().unwrap();
    w.replace(METADATA, vec![9u8; 12_345]).unwrap();
    let once = w.finish().unwrap();
    let twice = Abf::parse(&once).unwrap().to_writer().unwrap().finish().unwrap();
    assert!(once == twice);
}

#[test]
fn replace_errors() {
    let input = synth::build(&synth::sample_files(), &Options::default());
    let abf = Abf::parse(&input).unwrap();
    let mut w = abf.to_writer().unwrap();
    assert_eq!(w.len(), abf.files().len());
    assert_eq!(w.replace("missing.idf", vec![]), Err(Error::FileNotFound));
    assert!(matches!(w.replace("LOG", vec![]), Err(Error::Unsupported(_))));
    // Lookups as in `Abf::entry`: case, separators, storage names.
    w.replace(r"SALES (12).TBL\1.Sales (12).Amount (14).hidx", vec![1])
        .unwrap();
    let storage = abf.entry("0.CryptKey.bin").unwrap().storage_path.clone();
    w.replace(&storage.to_ascii_lowercase(), vec![2]).unwrap();
    let back_bytes = w.finish().unwrap();
    let back = check_consistent(&back_bytes);
    assert_eq!(back.read("0.CryptKey.bin"), Some(&[2u8][..]));

    // Excel flavor: the CRC end marker format is unknown, so nothing can be replaced.
    let crc = synth::build(
        &synth::sample_files(),
        &Options {
            crc_markers: true,
            ..Options::default()
        },
    );
    let crc_abf = Abf::parse(&crc).unwrap();
    let mut w = crc_abf.to_writer().unwrap();
    assert!(matches!(w.replace(METADATA, vec![]), Err(Error::Unsupported(_))));

    // No metadata.sqlitedb.
    let empty = synth::build(&[], &Options::default());
    let empty_abf = Abf::parse(&empty).unwrap();
    assert_eq!(
        empty_abf.to_writer().unwrap().replace_metadata_sqlitedb(vec![]),
        Err(Error::FileNotFound)
    );
}

#[test]
fn ambiguous_paths_are_rejected() {
    let files: Vec<(&str, &[u8])> = vec![
        ("T.tbl/Dup.idf", b"a"),
        ("T.tbl/dup.idf", b"b"),
        ("A/metadata.sqlitedb", b"m1"),
        ("B/metadata.sqlitedb", b"m2"),
    ];
    let input = synth::build(&files, &Options::default());
    let abf = Abf::parse(&input).unwrap();
    let mut w = abf.to_writer().unwrap();
    assert_eq!(w.replace("T.TBL/DUP.IDF", vec![]), Err(Error::AmbiguousPath(2)));
    assert_eq!(w.replace_metadata_sqlitedb(vec![]), Err(Error::AmbiguousPath(2)));
    // An exact match is unambiguous.
    w.replace("T.tbl/dup.idf", b"c".to_vec()).unwrap();
    w.replace("B/metadata.sqlitedb", b"m3".to_vec()).unwrap();
    let out = w.finish().unwrap();
    let back = check_consistent(&out);
    assert_eq!(back.read("T.tbl/Dup.idf"), Some(&b"a"[..]));
    assert_eq!(back.read("T.tbl/dup.idf"), Some(&b"c"[..]));
    assert_eq!(back.read("B/metadata.sqlitedb"), Some(&b"m3"[..]));
}

#[test]
fn hostile_sizes_are_rejected() {
    assert!(matches!(file_offsets(&[u64::MAX]), Err(Error::Write(_))));
    assert!(file_offsets(&[u64::MAX - PAGE_SIZE as u64]).is_ok());
    assert!(file_offsets(&[u64::MAX - PAGE_SIZE as u64, 1]).is_err());
    assert!(file_offsets(&[u64::MAX / 2, u64::MAX / 2]).is_err());
    assert!(matches!(align_page(u64::MAX), Err(Error::Write(_))));
    assert_eq!(
        align_page(u64::MAX - (PAGE_SIZE as u64 - 1)).ok(),
        Some(u64::MAX - 4095)
    );
    assert_eq!(align_page(0).ok(), Some(0));
    assert_eq!(align_page(4097).ok(), Some(8192));
    let (offsets, end) = file_offsets(&[1, 0, 2]).unwrap();
    assert_eq!((offsets, end), (vec![4096, 4097, 4097], 4099));
}

#[test]
fn writer_never_panics_on_corrupt_input() {
    for opts in flavors() {
        let bytes = synth::build(&synth::sample_files(), &opts);
        let mut state = 0xD1B5_4A32_D192_ED03u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let (mut parsed, mut written) = (0, 0);
        for round in 0..1500 {
            let mut corrupt = bytes.clone();
            for _ in 0..1 + next() % 4 {
                let i = (next() % corrupt.len() as u64) as usize;
                corrupt[i] = (next() & 0xFF) as u8;
            }
            let Ok(abf) = Abf::parse(&corrupt) else { continue };
            parsed += 1;
            let Ok(mut w) = abf.to_writer() else { continue };
            if round % 2 == 0 {
                let _ = w.replace_metadata_sqlitedb(vec![0u8; (next() % 9000) as usize]);
            }
            if let Ok(out) = w.finish() {
                written += 1;
                assert!(Abf::parse(&out).is_ok());
            }
        }
        assert!(parsed > 0 && written > 0, "{parsed} parsed, {written} written");
    }
}

// --- add and remove ----------------------------------------------------------------------------

const AMOUNT_IDF: &str = "Sales (12).tbl/13.prt/1.Sales (12).Amount (14).0.idf";

#[test]
fn removing_a_file_drops_its_entry_and_record() {
    let input = synth::build(&synth::sample_files(), &Options::default());
    let abf = Abf::parse(&input).unwrap();
    let mut w = abf.to_writer().unwrap();
    w.remove(AMOUNT_IDF).unwrap();
    assert_eq!(w.len(), abf.files().len() - 1);
    assert_eq!(w.remove(AMOUNT_IDF), Err(Error::FileNotFound), "already removed");
    let out = w.finish().unwrap();
    let back = check_consistent(&out);
    assert_eq!(back.files().len(), abf.files().len() - 1);
    assert!(back.entry(AMOUNT_IDF).is_none());
    let records: usize = back.backup_log().file_groups.iter().map(|g| g.files.len()).sum();
    let before: usize = abf.backup_log().file_groups.iter().map(|g| g.files.len()).sum();
    assert_eq!(records, before - 1);
    for f in abf
        .files()
        .iter()
        .filter(|f| f.path != AMOUNT_IDF && f.kind != FileKind::BackupLog)
    {
        assert_eq!(back.read(&f.path), abf.data(f), "{} kept", f.path);
    }
    // Writing the result again is stable.
    let again = Abf::parse(&out).unwrap().to_writer().unwrap().finish().unwrap();
    assert!(again == out);
}

#[test]
fn removing_every_database_file_leaves_a_valid_backup() {
    let input = synth::build(&synth::sample_files(), &Options::default());
    let abf = Abf::parse(&input).unwrap();
    let mut w = abf.to_writer().unwrap();
    for f in abf.files() {
        if f.kind != FileKind::BackupLog {
            w.remove(&f.storage_path).unwrap();
        }
    }
    assert_eq!(w.len(), 1);
    let out = w.finish().unwrap();
    let back = check_consistent(&out);
    assert_eq!(back.files().len(), 1);
    assert!(back.backup_log().file_groups.iter().all(|g| g.files.is_empty()));
}

#[test]
fn adding_files_places_them_before_the_metadata() {
    let files: Vec<(&str, &[u8])> = vec![
        ("0.CryptKey.bin", b"\x01"),
        ("T (1).tbl/2.prt/1.T (1).A (3).0.idf", b"a-idf"),
        ("metadata.sqlitedb", b"SQLite format 3\0x"),
    ];
    let input = synth::build(&files, &Options::default());
    let abf = Abf::parse(&input).unwrap();
    let mut w = abf.to_writer().unwrap();
    let new_idf = "T (1).tbl/2.prt/1.T (1).B & C (4).0.idf";
    let new_dict = r"T (1).tbl\1.T (1).B & C (4).dictionary";
    w.add(new_idf, b"new idf".to_vec()).unwrap();
    w.add(new_dict, vec![7u8; 5000]).unwrap();
    assert_eq!(w.len(), abf.files().len() + 2);
    let out = w.finish().unwrap();
    let back = check_consistent(&out);
    assert_eq!(back.read(new_idf), Some(&b"new idf"[..]));
    assert_eq!(
        back.read("T (1).tbl/1.T (1).B & C (4).dictionary"),
        Some(&[7u8; 5000][..])
    );
    for f in abf.files().iter().filter(|f| f.kind != FileKind::BackupLog) {
        assert_eq!(back.read(&f.path), abf.data(f));
    }
    // Order: ..., the new files in the order added, metadata.sqlitedb, LOG.
    let names: Vec<&str> = back.files().iter().map(|f| f.path.as_str()).collect();
    let n = names.len();
    assert_eq!(
        &names[n - 4..n - 2],
        &[new_idf, "T (1).tbl/1.T (1).B & C (4).dictionary"]
    );
    assert_eq!(names[n - 2], METADATA);
    // The new records carry an absolute path under the group's folder and a GUID-like name.
    let added = back.entry(new_idf).unwrap();
    let abs = added.original_path.as_deref().unwrap();
    assert!(abs.starts_with(synth::PERSIST_ROOT));
    assert!(abs.ends_with(r"\2.prt\1.T (1).B & C (4).0.idf"));
    assert_eq!(added.storage_path.len(), 20);
    assert!(
        added
            .storage_path
            .chars()
            .all(|c| c.is_ascii_digit() || ('A'..='F').contains(&c))
    );
    assert_eq!(&added.storage_path[12..13], "4");
    assert!("89AB".contains(&added.storage_path[16..17]));
    assert_eq!(added.file_group_class, Some(100069));
    assert_eq!(added.last_write_time, Some(134000000000000000));
    // Deterministic, and stable when written again.
    let mut w2 = abf.to_writer().unwrap();
    w2.add(new_idf, b"new idf".to_vec()).unwrap();
    w2.add(new_dict, vec![7u8; 5000]).unwrap();
    assert!(w2.finish().unwrap() == out);
    let again = Abf::parse(&out).unwrap().to_writer().unwrap().finish().unwrap();
    assert!(again == out);
}

#[test]
fn add_remove_and_replace_combine() {
    let input = synth::build(&synth::sample_files(), &Options::default());
    let abf = Abf::parse(&input).unwrap();
    let mut w = abf.to_writer().unwrap();
    // Remove then add the same path: a new file with a new storage name.
    w.remove(AMOUNT_IDF).unwrap();
    w.add(AMOUNT_IDF, b"rewritten".to_vec()).unwrap();
    // An added file can be replaced and removed through its path.
    let x = "X (90).tbl/1.X (90).Y (91).dictionary";
    let z = "Z (92).tbl/1.Z (92).Q (93).dictionary";
    w.add(x, b"tmp".to_vec()).unwrap();
    w.replace(&x.to_ascii_uppercase(), b"final".to_vec()).unwrap();
    w.add(z, b"gone".to_vec()).unwrap();
    w.remove(z).unwrap();
    w.replace_metadata_sqlitedb(vec![3u8; 10_000]).unwrap();
    assert_eq!(w.len(), abf.files().len() + 1);
    let out = w.finish().unwrap();
    let back = check_consistent(&out);
    assert_eq!(back.read(AMOUNT_IDF), Some(&b"rewritten"[..]));
    assert_ne!(
        back.entry(AMOUNT_IDF).unwrap().storage_path,
        abf.entry(AMOUNT_IDF).unwrap().storage_path
    );
    assert_eq!(back.read(x), Some(&b"final"[..]));
    assert!(back.entry(z).is_none());
    assert_eq!(back.metadata_sqlitedb(), Some(&[3u8; 10_000][..]));
}

#[test]
fn add_and_remove_errors() {
    let input = synth::build(&synth::sample_files(), &Options::default());
    let abf = Abf::parse(&input).unwrap();
    let mut w = abf.to_writer().unwrap();
    assert_eq!(
        w.add(&AMOUNT_IDF.to_ascii_uppercase(), vec![]),
        Err(Error::PathExists)
    );
    assert_eq!(w.add(METADATA, vec![]), Err(Error::PathExists));
    for bad in [
        "", "/abs.idf", "a//b", "a/../b", "./a", "a/", "C:/x", "a\u{0}b", "LOG",
    ] {
        assert!(
            matches!(w.add(bad, vec![]), Err(Error::Unsupported(_))),
            "{bad:?}"
        );
    }
    assert!(matches!(w.remove("LOG"), Err(Error::Unsupported(_))));
    assert_eq!(w.remove("nope.idf"), Err(Error::FileNotFound));
    assert_eq!(w.len(), abf.files().len(), "failed calls change nothing");
    assert!(w.finish().unwrap() == input);

    let crc = synth::build(
        &synth::sample_files(),
        &Options {
            crc_markers: true,
            ..Options::default()
        },
    );
    let crc_abf = Abf::parse(&crc).unwrap();
    let mut w = crc_abf.to_writer().unwrap();
    assert!(matches!(w.remove(AMOUNT_IDF), Err(Error::Unsupported(_))));
    assert!(matches!(w.add("new.idf", vec![]), Err(Error::Unsupported(_))));
}

#[test]
fn adding_to_a_backup_without_metadata_appends_to_the_last_group() {
    let input = synth::build(&[], &Options::default());
    let abf = Abf::parse(&input).unwrap();
    let mut w = abf.to_writer().unwrap();
    w.add("only.tbl/1.only.dictionary", b"x".to_vec()).unwrap();
    let out = w.finish().unwrap();
    let back = check_consistent(&out);
    assert_eq!(back.read("only.tbl/1.only.dictionary"), Some(&b"x"[..]));
    let names: Vec<&str> = back.files().iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        names,
        [
            "ADDITIONAL_LOG",
            "PARTITIONS",
            "only.tbl/1.only.dictionary",
            "LOG"
        ]
    );

    // UTF-16LE directory, no Power BI bookkeeping files, non-ASCII names.
    let opts = Options {
        dir_utf16: true,
        power_bi_bookkeeping: false,
        ..Options::default()
    };
    let input = synth::build(&synth::sample_files(), &opts);
    let abf = Abf::parse(&input).unwrap();
    let mut w = abf.to_writer().unwrap();
    let unicode = "\u{e9} (1).tbl/1.\u{e9} (1).\u{fc} (2).dictionary";
    w.add(unicode, b"unicode".to_vec()).unwrap();
    w.remove(AMOUNT_IDF).unwrap();
    let out = w.finish().unwrap();
    let back = check_consistent(&out);
    assert_eq!(back.read(unicode), Some(&b"unicode"[..]));
}

#[test]
fn storage_names_are_unique_and_guid_shaped() {
    let input = synth::build(&synth::sample_files(), &Options::default());
    let abf = Abf::parse(&input).unwrap();
    let mut w = abf.to_writer().unwrap();
    for i in 0..300u32 {
        w.add(&format!("t.tbl/{i}.idf"), vec![(i % 251) as u8]).unwrap();
    }
    let out = w.finish().unwrap();
    let back = check_consistent(&out);
    let mut names: Vec<&str> = back.files().iter().map(|f| f.storage_path.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), back.files().len());
    let name = crate::writer::storage_name("a/b.idf", 0);
    assert_eq!(name, crate::writer::storage_name("A/B.IDF", 0));
    assert_ne!(name, crate::writer::storage_name("a/b.idf", 1));
}

#[test]
fn editing_writer_never_panics_on_corrupt_input() {
    let bytes = synth::build(&synth::sample_files(), &Options::default());
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut written = 0;
    for _ in 0..1500 {
        let mut corrupt = bytes.clone();
        for _ in 0..1 + next() % 4 {
            let i = (next() % corrupt.len() as u64) as usize;
            corrupt[i] = (next() & 0xFF) as u8;
        }
        let Ok(abf) = Abf::parse(&corrupt) else { continue };
        let Ok(mut w) = abf.to_writer() else { continue };
        let paths: Vec<String> = abf.files().iter().map(|f| f.path.clone()).collect();
        if let Some(p) = paths.get((next() % paths.len().max(1) as u64) as usize) {
            let _ = w.remove(p);
        }
        let size = (next() % 5000) as usize;
        let _ = w.add("new (1).tbl/1.new (1).c (2).dictionary", vec![1u8; size]);
        if let Ok(out) = w.finish() {
            written += 1;
            assert!(Abf::parse(&out).is_ok());
        }
    }
    assert!(written > 0);
}
