//! Optional checks against real Power BI Desktop output kept outside the repository.
//!
//! Set `BICRAFT_TESTDATA` to the directory holding the samples (any local
//! folder outside the repository).
//! Each sample is skipped with a note when absent. Only structural facts are printed, never
//! sample content.
#![allow(clippy::indexing_slicing)]

use std::path::PathBuf;

use bicraft_xpress9::{
    DataModelInfo, decompress_datamodel, decompress_datamodel_reference, decompress_datamodel_with,
    inspect_datamodel, verify_datamodel,
};

/// Samples: (relative path, description).
const SAMPLES: [(&str, &str); 2] = [
    ("sample1/cache.abf", "PBIP cache.abf"),
    ("sample1/DataModel", "PBIX DataModel part"),
];

const LIMIT: usize = 4 << 30;

fn testdata_dir() -> Option<PathBuf> {
    match bicraft_env::var_os("BICRAFT_TESTDATA") {
        Some(d) => Some(PathBuf::from(d)),
        None => {
            eprintln!("note: BICRAFT_TESTDATA is not set; skipping real-sample checks");
            None
        }
    }
}

/// [MS-XLDM] §2.1.1: the header page starts with a UTF-16LE BOM and the stream storage signature.
fn is_abf(bytes: &[u8]) -> bool {
    let sig: Vec<u8> = "STREAM_STORAGE_SIGNATURE"
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    bytes.len() >= 4096 && bytes[..2] == [0xff, 0xfe] && bytes[2..2 + sig.len()] == sig[..]
}

fn describe(name: &str, part_len: usize, info: &DataModelInfo) {
    eprintln!(
        "{name}: {part_len} bytes, layout {:?}, sessions {}",
        info.layout, info.sessions
    );
    if let Some(h) = info.multithreaded_header {
        eprintln!("  multithreaded header: {h:?}");
    }
    let blocks: usize = info.chunks.iter().map(|c| c.blocks.len()).sum();
    eprintln!(
        "  chunks {}, blocks {}, total uncompressed {}, trailing bytes {}",
        info.chunks.len(),
        blocks,
        info.total_uncompressed,
        info.trailing_bytes
    );
    for (i, c) in info.chunks.iter().enumerate() {
        let b = c.blocks.first();
        eprintln!(
            "  chunk {i}: session {} uncompressed {} compressed {} blocks {} window 2^{} mtf {} ptr_min {} mtf_min {} \
             table_bits {} block_index {}",
            c.session,
            c.uncompressed_size,
            c.compressed_size,
            c.blocks.len(),
            b.map_or(0, |b| b.window_size_log2()),
            b.map_or(0, |b| b.mtf_entry_count()),
            b.map_or(0, |b| b.ptr_min_match_length()),
            b.map_or(0, |b| b.mtf_min_match_length()),
            b.map_or(0, |b| b.huffman_table_bits()),
            b.map_or(0, |b| b.block_index),
        );
    }
}

/// Does the ported encoder reproduce Desktop's compressed chunks (ignoring the random session
/// signature and the header CRC)? Tries the [`EncoderParams::power_bi_desktop`] preset first,
/// then scans lookup depths, optimisation levels and the SSE2 flag. Informational only.
fn report_reencoding(part: &[u8], info: &DataModelInfo, abf: &[u8]) {
    use bicraft_xpress9::{Encoder, EncoderParams, FLAG_USE_SSE2, Layout};
    if info.layout != Layout::SingleThreaded || info.chunks.is_empty() {
        return;
    }
    let first = info.chunks[0].blocks[0];
    let strip = |b: &[u8]| -> Vec<u8> {
        // blank header words 5 (session signature) and 7 (CRC) of every block
        let mut v = b.to_vec();
        let mut at = 0;
        while at + 32 <= v.len() {
            let Ok(h) = bicraft_xpress9::BlockHeader::parse(&b[at..]) else {
                break;
            };
            v[at + 20..at + 24].fill(0);
            v[at + 28..at + 32].fill(0);
            at += h.block_size();
        }
        v
    };
    let payloads: Vec<Vec<u8>> = info
        .chunks
        .iter()
        .map(|c| strip(&part[c.offset + 8..c.offset + 8 + c.compressed_size as usize]))
        .collect();
    let reproduces = |params: &EncoderParams, flags: u32| -> bool {
        let Ok(mut enc) = Encoder::with_flags(params.window_size_log2, flags) else {
            return false;
        };
        if enc.start_session(params, 0).is_err() {
            return false;
        }
        let mut at = 0usize;
        for (c, expected) in info.chunks.iter().zip(&payloads) {
            let n = c.uncompressed_size as usize;
            match enc.compress(&abf[at..at + n], true) {
                Ok(out) if strip(&out) == *expected => at += n,
                _ => return false,
            }
        }
        true
    };

    let (preset, preset_flags) = EncoderParams::power_bi_desktop();
    if reproduces(&preset, preset_flags) {
        eprintln!("  re-encoding with EncoderParams::power_bi_desktop() reproduces the stored bytes exactly");
        return;
    }
    let mut matches = Vec::new();
    for flags in [0, FLAG_USE_SSE2] {
        for opt in [0u32, 1] {
            for depth in 0..=9u32 {
                let params = EncoderParams {
                    max_stream_length: 0,
                    mtf_entry_count: first.mtf_entry_count(),
                    lookup_depth: depth,
                    window_size_log2: first.window_size_log2(),
                    optimization_level: opt,
                    ptr_min_match_length: first.ptr_min_match_length(),
                    mtf_min_match_length: first.mtf_min_match_length(),
                };
                if reproduces(&params, flags) {
                    matches.push((depth, opt, flags));
                }
            }
        }
    }
    eprintln!("  re-encoding: the preset does not match; matching (depth, opt, flags) settings: {matches:?}");
}

/// The exact mode (`CompressionMode::Exact` with the plan `compress_datamodel(abf, Some(&info))`
/// uses) must give Desktop's part back, with the same banner, envelopes and block headers; only
/// each block's session signature (word 5) and header CRC (word 7) may differ.
fn check_reframing(rel: &str, part: &[u8], info: &DataModelInfo, abf: &[u8]) {
    use bicraft_xpress9::datamodel::compress_with_plan;
    use bicraft_xpress9::{CompressionMode, FramingPlan, Layout};
    if info.layout != Layout::SingleThreaded {
        eprintln!("  {rel}: not single-threaded; re-framing comparison skipped");
        return;
    }
    let plan = FramingPlan::new(Some(info)).with_mode(CompressionMode::Exact);
    let ours = compress_with_plan(abf, &plan).unwrap_or_else(|e| panic!("{rel}: compress: {e}"));
    assert_eq!(
        decompress_datamodel(&ours, LIMIT).unwrap(),
        abf,
        "{rel}: round trip"
    );
    let again = inspect_datamodel(&ours).unwrap();
    assert_eq!(again.banner, info.banner, "{rel}: banner");
    assert_eq!(ours.len(), part.len(), "{rel}: part size");
    assert_eq!(again.chunks.len(), info.chunks.len(), "{rel}: chunk count");
    let mut differing = Vec::new();
    for (a, b) in again.chunks.iter().zip(&info.chunks) {
        assert_eq!(
            (a.offset, a.uncompressed_size, a.compressed_size, a.blocks.len()),
            (b.offset, b.uncompressed_size, b.compressed_size, b.blocks.len()),
            "{rel}: chunk envelope"
        );
        for (x, y) in a.blocks.iter().zip(&b.blocks) {
            assert_eq!(
                (x.original_size, x.encoded_size_bits, x.flags, x.block_index),
                (y.original_size, y.encoded_size_bits, y.flags, y.block_index),
                "{rel}: block header"
            );
        }
        let mut at = a.offset + 8;
        for x in &a.blocks {
            // words 5 (signature) and 7 (CRC) are allowed to differ
            for w in [20, 28] {
                differing.push(at + w);
            }
            at += x.block_size();
        }
    }
    let mask = |v: &[u8]| -> Vec<u8> {
        let mut v = v.to_vec();
        for &at in &differing {
            v[at..at + 4].fill(0);
        }
        v
    };
    assert!(
        mask(&ours) == mask(part),
        "{rel}: re-framed bytes differ beyond signature and CRC"
    );
    eprintln!("  re-framing in CompressionMode::Exact matches Desktop's part (signature and CRC aside)");
}

#[test]
fn real_desktop_samples_decompress_to_abf() {
    let Some(dir) = testdata_dir() else { return };
    let mut abfs: Vec<(&str, Vec<u8>)> = Vec::new();
    for (rel, what) in SAMPLES {
        let path = dir.join(rel);
        let Ok(part) = std::fs::read(&path) else {
            eprintln!("note: {what} sample {rel} not found under BICRAFT_TESTDATA; skipping");
            continue;
        };
        let info = inspect_datamodel(&part).unwrap_or_else(|e| panic!("{rel}: framing: {e}"));
        describe(rel, part.len(), &info);
        let abf = decompress_datamodel(&part, LIMIT).unwrap_or_else(|e| panic!("{rel}: decompress: {e}"));
        assert_eq!(abf.len() as u64, info.total_uncompressed, "{rel}");
        // the fast decoder takes Desktop's stream as is, on any thread count, and gives the
        // reference decoder's bytes; the parallel check accepts them
        assert!(
            bicraft_xpress9::datamodel::decodes_on_fast_path(&part),
            "{rel}: the fast decoder fell back to the reference decoder"
        );
        assert!(
            abf == decompress_datamodel_reference(&part, LIMIT).unwrap(),
            "{rel}: fast vs reference"
        );
        for threads in [1, 3] {
            assert!(
                abf == decompress_datamodel_with(&part, LIMIT, threads).unwrap(),
                "{rel}: {threads} threads"
            );
        }
        assert!(verify_datamodel(&part, &abf).unwrap(), "{rel}: verify");
        assert!(
            is_abf(&abf),
            "{rel}: decompressed data does not start with the ABF stream storage signature"
        );
        eprintln!(
            "  -> ABF of {} bytes ({} pages of 4 KiB)",
            abf.len(),
            abf.len().div_ceil(4096)
        );
        report_reencoding(&part, &info, &abf);
        check_reframing(rel, &part, &info, &abf);
        abfs.push((rel, abf));
    }
    if let [(a_name, a), (b_name, b)] = &abfs[..] {
        let common = a.len().min(b.len());
        let equal = a.iter().zip(b.iter()).filter(|(x, y)| x == y).count();
        let prefix = a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count();
        let pages_equal = a.chunks(4096).zip(b.chunks(4096)).filter(|(x, y)| x == y).count();
        eprintln!(
            "compare {a_name} vs {b_name}: sizes {} / {}, identical {}, common prefix {prefix} bytes, \
             {equal} of {common} aligned bytes equal, {pages_equal} of {} aligned 4 KiB pages identical",
            a.len(),
            b.len(),
            a == b,
            common.div_ceil(4096),
        );
    }
}
