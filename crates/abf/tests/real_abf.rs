//! Local-only checks against real backups. Nothing here is committed data.
//!
//! - `BICRAFT_ABF`: one or more already-decompressed `.abf` streams (path-list syntax of the
//!   platform, e.g. `a.abf;b.abf` on Windows). Each must parse and expose `metadata.sqlitedb`.
//!   Each is also re-written: unchanged it must come out byte-identical, and with
//!   `metadata.sqlitedb` replaced only that file (and the logs) may change.
//! - `BICRAFT_TESTDATA`: the local test-data folder. Every `DataModel` / `*.abf` found in it
//!   is classified: still XPress9-compressed parts must be rejected with
//!   `Error::Xpress9Compressed`, bare ABFs must parse.
//!
//! Output is limited to counts (`Summary`), never names or content.

use std::path::{Path, PathBuf};

use bicraft_abf::{Abf, Error, FileKind, PAGE_SIZE};

const MAX_INPUT: u64 = 1 << 30;

fn read_capped(path: &Path) -> Option<Vec<u8>> {
    let len = std::fs::metadata(path).ok()?.len();
    if len > MAX_INPUT {
        eprintln!("note: skipping a {len}-byte input (over the test cap)");
        return None;
    }
    std::fs::read(path).ok()
}

fn check_parsed(abf: &Abf<'_>, len: usize) {
    let mut total = 0usize;
    for f in abf.files() {
        let data = abf.data(f).expect("entry data in bounds");
        assert_eq!(data.len(), f.size);
        assert!(f.size <= f.stored_size);
        total += f.stored_size;
    }
    assert!(total <= len);
    let sqlite = abf.metadata_sqlitedb().expect("metadata.sqlitedb present");
    assert!(
        sqlite.starts_with(b"SQLite format 3\0"),
        "metadata.sqlitedb is not SQLite"
    );
    assert_eq!(abf.files().last().map(|f| f.kind), Some(FileKind::BackupLog));
}

/// Structural invariants of a written stream (public API only).
fn check_written(bytes: &[u8]) -> Abf<'_> {
    let abf = Abf::parse(bytes).expect("written ABF parses");
    assert_eq!(bytes.len() % PAGE_SIZE, 0);
    let h = abf.header();
    assert_eq!(h.file_count, Some(abf.files().len() as u64));
    let mut pos = PAGE_SIZE;
    for f in abf.files() {
        assert_eq!(f.offset, pos, "files are contiguous");
        pos += f.stored_size;
    }
    assert_eq!(h.directory_offset as usize, pos.next_multiple_of(PAGE_SIZE));
    let dir_end = (h.directory_offset + h.directory_size) as usize;
    assert_eq!(bytes.len(), dir_end.next_multiple_of(PAGE_SIZE));
    for rec in abf.backup_log().file_groups.iter().flat_map(|g| &g.files) {
        let e = abf.entry(&rec.storage_path).expect("log record has a file");
        assert_eq!(rec.size, Some(e.size as u64), "log Size matches the file");
    }
    assert_eq!(abf.summary().unmatched_log_records, 0);
    abf
}

/// Identity must be byte-exact; replacing `metadata.sqlitedb` (same size and different
/// sizes) must change that file only.
fn round_trip_real(abf: &Abf<'_>, bytes: &[u8]) {
    let out = abf.to_writer().expect("writer").finish().expect("identity write");
    assert_eq!(out.len(), bytes.len(), "identity length");
    assert!(out == bytes, "identity round trip is not byte-identical");

    let original = abf.metadata_sqlitedb().expect("metadata.sqlitedb");
    let mut flipped = original.to_vec();
    for b in flipped.iter_mut().skip(100).step_by(997) {
        *b ^= 0x5A;
    }
    let mut grown = original.to_vec();
    grown.extend(std::iter::repeat_n(0xC3, 3 * PAGE_SIZE + 123));
    let shrunk = original.get(..original.len() / 3).unwrap_or_default().to_vec();
    for new in [flipped, grown, shrunk] {
        let mut w = abf.to_writer().expect("writer");
        w.replace_metadata_sqlitedb(new.clone()).expect("replace");
        let out = w.finish().expect("write");
        let back = check_written(&out);
        assert_eq!(back.metadata_sqlitedb(), Some(&new[..]));
        assert_eq!(back.files().len(), abf.files().len());
        let target = abf.metadata_entry().map(|e| e.storage_path.clone());
        for (old, newf) in abf.files().iter().zip(back.files()) {
            assert_eq!(old.path, newf.path);
            assert_eq!(old.last_write_time, newf.last_write_time);
            if Some(&old.storage_path) != target.as_ref() && old.kind != FileKind::BackupLog {
                assert!(back.data(newf) == abf.data(old), "an untouched file changed");
            }
        }
        if new.len() == original.len() {
            assert_eq!(out.len(), bytes.len());
            let diff = out.iter().zip(bytes).filter(|(a, b)| a != b).count();
            assert!(
                diff > 0 && diff <= new.len(),
                "same-size replacement touched other bytes"
            );
        }
    }
    eprintln!("writer: identity byte-exact; metadata.sqlitedb replacements consistent");
}

/// Removing the files of one VertiPaq table folder and adding new ones: every other file keeps
/// its bytes, the BackupLog and directory stay consistent, and the added files sit before
/// `metadata.sqlitedb` in the group that holds it.
fn add_remove_real(abf: &Abf<'_>) {
    let Some(victim) = abf
        .files()
        .iter()
        .find(|f| f.kind == FileKind::Dictionary)
        .map(|f| f.path.clone())
    else {
        eprintln!("note: no dictionary file to remove; add/remove check skipped");
        return;
    };
    let folder = victim.rsplit_once('/').map_or("", |(d, _)| d).to_string();
    let removed: Vec<String> = abf
        .files()
        .iter()
        .filter(|f| !folder.is_empty() && f.path.starts_with(&format!("{folder}/")))
        .map(|f| f.path.clone())
        .collect();
    let mut w = abf.to_writer().expect("writer");
    for p in &removed {
        w.remove(p).expect("remove");
    }
    let added = [
        (
            format!("{folder}/1.bicraft (1).new (2).dictionary"),
            vec![0x11u8; 777],
        ),
        (
            format!("{folder}/9.prt/1.bicraft (1).new (2).0.idf"),
            vec![0x22u8; 64],
        ),
    ];
    for (p, data) in &added {
        w.add(p, data.clone()).expect("add");
    }
    let out = w.finish().expect("write");
    let back = check_written(&out);
    assert_eq!(
        back.files().len(),
        abf.files().len() - removed.len() + added.len()
    );
    for p in &removed {
        assert!(back.entry(p).is_none(), "a removed file is still there");
    }
    for (p, data) in &added {
        let e = back.entry(p).expect("added file present");
        assert_eq!(back.data(e), Some(&data[..]));
        let meta = abf.metadata_entry().expect("metadata entry");
        assert_eq!(e.file_group_class, meta.file_group_class);
    }
    for f in abf.files().iter().filter(|f| f.kind != FileKind::BackupLog) {
        if !removed.contains(&f.path) {
            assert!(back.read(&f.path) == abf.data(f), "an untouched file changed");
        }
    }
    let n = back.files().len();
    assert_eq!(back.files()[n - 2].kind, FileKind::MetadataSqlite);
    eprintln!(
        "writer: removed {} file(s) of one folder, added {}; consistent",
        removed.len(),
        added.len()
    );
}

/// Sampled corruption of a real stream: must never panic.
fn corrupt_real(bytes: &[u8]) {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut copy = bytes.to_vec();
    for _ in 0..200 {
        let i = (next() % bytes.len() as u64) as usize;
        copy[i] ^= 0xFF;
        if let Ok(a) = Abf::parse(&copy) {
            let _ = a.summary();
            let _ = a.to_writer().and_then(|w| w.finish());
        }
        copy[i] = bytes[i];
    }
    for _ in 0..50 {
        let cut = (next() % bytes.len() as u64) as usize;
        let _ = Abf::parse(&bytes[..cut]).map(|a| a.summary());
    }
}

#[test]
fn decompressed_abfs_from_env() {
    let Some(list) = bicraft_env::var_os("BICRAFT_ABF") else {
        eprintln!("note: BICRAFT_ABF not set; skipping real-ABF checks");
        return;
    };
    for path in std::env::split_paths(&list) {
        let Some(bytes) = read_capped(&path) else {
            eprintln!("note: an entry of BICRAFT_ABF is missing or unreadable; skipped");
            continue;
        };
        let abf = Abf::parse(&bytes).expect("real ABF parses");
        check_parsed(&abf, bytes.len());
        eprintln!("{}\n", abf.summary());
        round_trip_real(&abf, &bytes);
        add_remove_real(&abf);
        corrupt_real(&bytes);
    }
}

fn collect_candidates(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth < 4 {
                collect_candidates(&path, depth + 1, out);
            }
        } else if out.len() < 200 {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            if name == "DataModel" || name.to_ascii_lowercase().ends_with(".abf") {
                out.push(path);
            }
        }
    }
}

#[test]
fn testdata_parts_are_classified() {
    let Some(root) = bicraft_env::var_os("BICRAFT_TESTDATA").map(PathBuf::from) else {
        eprintln!("note: BICRAFT_TESTDATA not set; skipping");
        return;
    };
    if !root.is_dir() {
        eprintln!("note: BICRAFT_TESTDATA is not a directory; skipping");
        return;
    }
    let mut candidates = Vec::new();
    collect_candidates(&root, 0, &mut candidates);
    let (mut compressed, mut parsed) = (0, 0);
    for path in candidates {
        let Some(bytes) = read_capped(&path) else { continue };
        match Abf::parse(&bytes) {
            Err(Error::Xpress9Compressed) => compressed += 1,
            Ok(abf) => {
                check_parsed(&abf, bytes.len());
                eprintln!("{}\n", abf.summary());
                parsed += 1;
            }
            Err(e) if bytes.starts_with(&[0xFF, 0xFE, b'S', 0]) => panic!("bare ABF failed to parse: {e}"),
            Err(_) => {}
        }
    }
    eprintln!("testdata: {compressed} XPress9-compressed part(s) recognized, {parsed} bare ABF(s) parsed");
}
