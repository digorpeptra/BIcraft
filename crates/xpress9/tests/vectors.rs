//! Test vectors produced by Microsoft's XPress9 C code (built with gcc as an oracle, see the
//! crate README): the Rust decoder must reproduce every input, and the Rust encoder must emit
//! the very same bytes as the C encoder.
#![allow(clippy::indexing_slicing)]

mod common;

use bicraft_xpress9::{
    DEFAULT_SESSION_SEED, Decoder, Encoder, EncoderParams, Error, Layout, compress, decompress,
    decompress_datamodel, inspect_datamodel,
};
use common::{SplitMix, generate, manifest, read_vector, rust_build};

#[test]
fn manifest_lists_all_vectors() {
    let m = manifest();
    assert!(m.len() >= 30, "manifest has {} entries", m.len());
    for v in &m {
        assert!(
            common::vectors_dir().join(format!("{}.x9", v.name)).exists(),
            "{} missing",
            v.name
        );
    }
}

#[test]
fn rust_decoder_reproduces_every_vector() {
    for v in manifest() {
        let input = generate(&v.gen_name, v.size, v.seed);
        let compressed = read_vector(&v);
        let out = match v.kind.as_str() {
            "stream" => decompress(&compressed, v.size),
            _ => decompress_datamodel(&compressed, v.size),
        }
        .unwrap_or_else(|e| panic!("{}: {e}", v.name));
        assert!(out == input, "{}: decoded bytes differ", v.name);
    }
}

#[test]
fn rust_encoder_matches_c_encoder_byte_for_byte() {
    for v in manifest() {
        let input = generate(&v.gen_name, v.size, v.seed);
        let expected = read_vector(&v);
        let actual = rust_build(&v, &input);
        if actual != expected {
            let first = actual.iter().zip(&expected).position(|(a, b)| a != b);
            panic!(
                "{}: Rust encoder output differs from the C oracle (len {} vs {}, first difference at {:?})",
                v.name,
                actual.len(),
                expected.len(),
                first
            );
        }
    }
}

#[test]
fn datamodel_framing_of_synthetic_parts() {
    for v in manifest().iter().filter(|v| v.kind.starts_with("dm_")) {
        let part = read_vector(v);
        let info = inspect_datamodel(&part).unwrap();
        assert_eq!(info.total_uncompressed, v.size as u64);
        assert_eq!(info.chunks.len(), v.size.div_ceil(v.chunk), "{}", v.name);
        assert!(info.chunks.iter().all(|c| !c.blocks.is_empty()));
        if v.kind == "dm_single" {
            assert_eq!(info.layout, Layout::SingleThreaded);
            assert_eq!(info.sessions, 1);
            // one session: block indices keep counting across chunks
            let indices: Vec<u32> = info
                .chunks
                .iter()
                .flat_map(|c| c.blocks.iter().map(|b| b.block_index))
                .collect();
            assert_eq!(indices, (0..indices.len() as u32).collect::<Vec<_>>());
        } else {
            assert_eq!(info.layout, Layout::MultiThreaded);
            let [pt, _, mt, _] = v.groups;
            assert_eq!(info.sessions as u64, pt + mt);
            assert_eq!(info.trailing_bytes, 0);
        }
        // limits are enforced
        assert_eq!(
            decompress_datamodel(&part, v.size - 1),
            Err(Error::LimitExceeded(v.size - 1))
        );
    }
}

#[test]
fn uncompressed_abf_passes_through() {
    let mut abf = vec![0xff, 0xfe];
    for u in "STREAM_STORAGE_SIGNATURE_)!@#$%^&*(".encode_utf16() {
        abf.extend(u.to_le_bytes());
    }
    abf.resize(4096, 0);
    assert_eq!(inspect_datamodel(&abf).unwrap().layout, Layout::Uncompressed);
    assert_eq!(decompress_datamodel(&abf, 1 << 20).unwrap(), abf);
    assert!(decompress_datamodel(&abf, 100).is_err());
}

#[test]
fn encoder_round_trips_many_parameter_sets() {
    let inputs = [
        generate("mixed", 150_000, 101),
        generate("text", 50_000, 102),
        generate("random", 5_000, 103),
        generate("struct", 70_000, 104),
        Vec::new(),
        vec![7u8; 1],
    ];
    let mut n = 0;
    for mtf in [0u32, 2, 4] {
        for ptr_min in [3u32, 4] {
            for mtf_min in [2u32, 3] {
                for (depth, opt) in [(0u32, 0u32), (1, 0), (3, 1), (9, 0), (12, 1)] {
                    for window in [16u32, 18] {
                        let params = EncoderParams {
                            max_stream_length: 0,
                            mtf_entry_count: mtf,
                            lookup_depth: depth,
                            window_size_log2: window,
                            optimization_level: opt,
                            ptr_min_match_length: ptr_min,
                            mtf_min_match_length: mtf_min,
                        };
                        let input = &inputs[n % inputs.len()];
                        n += 1;
                        let c = compress(input, &params).unwrap_or_else(|e| panic!("{params:?}: {e}"));
                        let d = decompress(&c, input.len()).unwrap_or_else(|e| panic!("{params:?}: {e}"));
                        assert!(d == *input, "{params:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn sse2_insertion_and_desktop_preset_round_trip() {
    let (params, flags) = EncoderParams::power_bi_desktop();
    for (i, input) in [
        generate("mixed", 400_000, 301),
        generate("struct", 300_000, 302),
        generate("text", 33, 303),
    ]
    .into_iter()
    .enumerate()
    {
        let mut enc = Encoder::with_flags(22, flags).unwrap();
        enc.start_session(&params, i as u64).unwrap();
        let c = enc.compress(&input, true).unwrap();
        assert_eq!(decompress(&c, input.len()).unwrap(), input);
        // the SSE2 hash changes the match search, not the format
        let mut plain = Encoder::new(22).unwrap();
        plain.start_session(&params, i as u64).unwrap();
        let p = plain.compress(&input, true).unwrap();
        assert_eq!(decompress(&p, input.len()).unwrap(), input);
    }
}

#[test]
fn streaming_sessions_carry_history_across_calls() {
    let input = generate("mixed", 300_000, 7);
    let params = EncoderParams {
        window_size_log2: 17,
        ..EncoderParams::default()
    };
    let mut enc = Encoder::new(17).unwrap();
    enc.start_session(&params, 42).unwrap();
    let mut pieces = Vec::new();
    // unflushed calls may leave data buffered; the final flush emits the rest
    for c in input.chunks(33_333) {
        pieces.push(enc.compress(c, false).unwrap());
    }
    pieces.push(enc.compress(&[], true).unwrap());
    let all: Vec<u8> = pieces.concat();
    assert_eq!(decompress(&all, input.len()).unwrap(), input);

    // decode block by block with a session decoder
    let mut dec = Decoder::new(22).unwrap();
    let mut out = Vec::new();
    let mut at = 0;
    while at < all.len() {
        at += dec.decode_block(&all[at..], &mut out, usize::MAX).unwrap();
    }
    assert_eq!(out, input);

    // a block out of order (or from another session) is rejected
    let first_len = bicraft_xpress9::BlockHeader::parse(&all).unwrap().block_size();
    if first_len < all.len() {
        assert!(decompress(&all[first_len..], input.len() - 1).is_err());
    }
}

#[test]
fn decompress_enforces_expected_length() {
    let input = generate("text", 10_000, 5);
    let c = compress(&input, &EncoderParams::default()).unwrap();
    assert_eq!(decompress(&c, 9_999), Err(Error::LimitExceeded(9_999)));
    assert_eq!(
        decompress(&c, 10_001),
        Err(Error::SizeMismatch {
            expected: 10_001,
            actual: 10_000
        })
    );
    assert_eq!(decompress(&[], 0).unwrap(), Vec::<u8>::new());
    assert!(decompress(&[], 1).is_err());
    assert!(Decoder::new(15).is_err());
    assert!(Decoder::new(23).is_err());
    let mut enc = Encoder::new(16).unwrap();
    assert!(enc.compress(b"x", true).is_err(), "no session");
    assert!(
        enc.start_session(&EncoderParams::default(), 0).is_err(),
        "window above the encoder maximum"
    );
}

#[test]
fn window_larger_than_decoder_maximum_is_rejected() {
    let input = generate("text", 5_000, 9);
    let c = compress(&input, &EncoderParams::default()).unwrap();
    let mut dec = Decoder::new(16).unwrap();
    let mut out = Vec::new();
    assert_eq!(
        dec.decode_block(&c, &mut out, usize::MAX),
        Err(Error::WindowTooLarge(22, 16))
    );
    // the failed session must be restarted
    assert!(dec.decode_block(&c, &mut out, usize::MAX).is_err());
    let mut dec = Decoder::new(22).unwrap();
    dec.start_session();
    dec.decode_block(&c, &mut out, usize::MAX).unwrap();
    assert_eq!(out, input);
}

#[test]
fn session_seed_sets_the_signature() {
    let input = generate("text", 1_000, 11);
    let mut enc = Encoder::new(16).unwrap();
    let params = EncoderParams {
        window_size_log2: 16,
        ..EncoderParams::default()
    };
    enc.start_session(&params, DEFAULT_SESSION_SEED).unwrap();
    let a = enc.compress(&input, true).unwrap();
    enc.start_session(&params, 99).unwrap();
    let b = enc.compress(&input, true).unwrap();
    assert_ne!(a[20..24], b[20..24]);
    assert_eq!(a[32..], b[32..]);
}

/// Truncated and corrupted inputs must give `Err` (or, for a harmless flip, some output),
/// never a panic, a hang or an unbounded allocation.
#[test]
fn corrupted_and_truncated_inputs_never_panic() {
    let m = manifest();
    let pick = [
        "one_byte",
        "text_100",
        "mixed_w16_65537",
        "p_mtf0_ptr3_d0",
        "p_mtf2_ptr3_mtf3_d2",
        "p_lazy_ptr3_d4",
        "random_16k",
        "chunked_w16_session",
        "dm_single",
        "dm_multi",
    ];
    let mut rng = SplitMix(0xfeed);
    let scale = fuzz_scale();
    for name in pick {
        let v = m.iter().find(|v| v.name == name).expect("vector");
        let data = read_vector(v);
        let run = |bytes: &[u8]| {
            let _ = if v.kind == "stream" {
                decompress(bytes, v.size)
            } else {
                decompress_datamodel(bytes, v.size)
            };
            let _ = inspect_datamodel(bytes);
        };
        // truncations
        let step = (data.len() / 300).max(1);
        for len in (0..data.len()).step_by(step) {
            run(&data[..len]);
        }
        // single bit flips, biased towards the header and Huffman tables
        for i in 0..400 * scale {
            let mut d = data.clone();
            let span = if i % 2 == 0 { d.len().min(400) } else { d.len() };
            let pos = (rng.next_u64() % span as u64) as usize;
            d[pos] ^= 1 << (rng.next_u64() % 8);
            run(&d);
        }
        // random byte overwrites past the header (header CRC stays valid)
        for _ in 0..200 * scale {
            let mut d = data.clone();
            for _ in 0..3 {
                let pos = (rng.next_u64() % d.len() as u64) as usize;
                d[pos] = rng.next_u64() as u8;
            }
            run(&d);
        }
    }
}

/// Iteration multiplier for the fuzz-style tests (`BICRAFT_FUZZ_SCALE`, default 1).
fn fuzz_scale() -> usize {
    bicraft_env::var("BICRAFT_FUZZ_SCALE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
}

/// Random payloads behind headers with a valid magic and CRC, so the decoder gets past the
/// header checks into the Huffman tables and the LZ77 loop.
#[test]
fn random_payloads_behind_valid_headers_never_panic() {
    let magic = bicraft_xpress9::XPRESS9_MAGIC;
    let mut rng = SplitMix(0xbad5eed);
    // real Huffman tables followed by garbage are the most interesting: take the first bytes of
    // a genuine block and randomise the rest
    let genuine = compress(&generate("mixed", 20_000, 77), &EncoderParams::default()).unwrap();
    for i in 0..3000 * fuzz_scale() {
        let payload_len = 8 + (rng.next_u64() % 600) as usize;
        let mut payload: Vec<u8> = (0..payload_len).map(|_| rng.next_u64() as u8).collect();
        if i % 2 == 0 {
            let keep = (rng.next_u64() % 400) as usize;
            for (d, s) in payload.iter_mut().zip(genuine[32..].iter().take(keep)) {
                *d = *s;
            }
        }
        let flags = (rng.next_u64() as u32) & 0x000f_ffff & !(3 << 16) | ((rng.next_u64() % 3) as u32) << 16;
        let flags = if i % 2 == 0 {
            (genuine[12] as u32 | (genuine[13] as u32) << 8 | (genuine[14] as u32) << 16) & 0xf_ffff
        } else {
            flags
        };
        let bits = 256 + (flags & 0x1fff) + 1 + (rng.next_u64() % (payload_len as u64 * 8)) as u32;
        let orig = (rng.next_u64() % 200_000) as u32;
        let words = [magic, orig, bits, flags, 0, 7, 0];
        let mut block: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let crc = bicraft_xpress9::xpress9_crc32(&block, 0);
        block.extend(crc.to_le_bytes());
        block.extend(&payload);
        let _ = decompress(&block, orig as usize);
    }
}

/// Hand-made hostile headers: valid magic and CRC, absurd sizes.
#[test]
fn hostile_headers_are_rejected() {
    fn header(words: [u32; 7]) -> Vec<u8> {
        let mut h: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let crc = bicraft_xpress9::xpress9_crc32(&h, 0);
        h.extend(crc.to_le_bytes());
        h
    }
    let magic = bicraft_xpress9::XPRESS9_MAGIC;
    // huge original size, tiny payload
    let mut b = header([magic, u32::MAX, 256 + 64, 0, 0, 1, 0]);
    b.extend([0u8; 8]);
    assert!(decompress(&b, 1 << 20).is_err());
    assert!(decompress(&b, usize::MAX >> 1).is_err());
    // encoded size claims more bytes than present
    let b = header([magic, 10, 8 * 4096, 0, 0, 1, 0]);
    assert_eq!(decompress(&b, 10), Err(Error::Truncated("block payload")));
    // reserved flags / MTF count 6 / window 2^23
    for flags in [1 << 20, 3 << 16, 7 << 13] {
        let mut b = header([magic, 10, 512, flags, 0, 1, 0]);
        b.extend([0u8; 64]);
        assert!(decompress(&b, 10).is_err(), "flags {flags:#x}");
    }
    // first block must have index 0
    let mut b = header([magic, 10, 512, 0, 0, 1, 1]);
    b.extend([0u8; 64]);
    assert_eq!(
        decompress(&b, 10),
        Err(Error::CorruptedHeader("unexpected block index"))
    );

    // DataModel framing: chunk counts larger than the part can hold
    let mut part = common::synthetic_banner(true);
    for x in [u64::MAX, 2, u64::MAX, 3, 1 << 21] {
        part.extend(x.to_le_bytes());
    }
    assert!(decompress_datamodel(&part, 1 << 30).is_err());
    let mut part = common::synthetic_banner(false);
    part.extend(u32::MAX.to_le_bytes());
    part.extend(u32::MAX.to_le_bytes());
    assert!(decompress_datamodel(&part, 1 << 30).is_err());
    assert!(decompress_datamodel(&[0u8; 10], 1 << 30).is_err());
}

/// Writes Rust-encoded streams for a one-off check with the C decoder
/// (`BICRAFT_XPRESS9_EXPORT=<dir> cargo test -p bicraft-xpress9 export_for_c_oracle`).
#[test]
fn export_for_c_oracle() {
    let Some(dir) = bicraft_env::var_os("BICRAFT_XPRESS9_EXPORT") else {
        eprintln!("BICRAFT_XPRESS9_EXPORT not set; skipping export");
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cases: [(&str, Vec<u8>, EncoderParams); 4] = [
        (
            "rust_mixed_default",
            generate("mixed", 500_000, 201),
            EncoderParams::default(),
        ),
        (
            "rust_text_lazy",
            generate("text", 300_000, 202),
            EncoderParams {
                optimization_level: 1,
                lookup_depth: 15,
                ..EncoderParams::default()
            },
        ),
        (
            "rust_struct_mtf0_ptr3",
            generate("struct", 400_000, 203),
            EncoderParams {
                mtf_entry_count: 0,
                ptr_min_match_length: 3,
                lookup_depth: 2,
                ..EncoderParams::default()
            },
        ),
        (
            "rust_mixed_w16",
            generate("mixed", 250_000, 204),
            EncoderParams {
                window_size_log2: 16,
                mtf_entry_count: 2,
                ..EncoderParams::default()
            },
        ),
    ];
    let mut list = String::new();
    for (name, input, params) in cases {
        let c = compress(&input, &params).unwrap();
        std::fs::write(dir.join(format!("{name}.x9")), &c).unwrap();
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in &input {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x100_0000_01b3);
        }
        list.push_str(&format!("{name} {} {h:016x}\n", input.len()));
    }
    std::fs::write(dir.join("list.txt"), list).unwrap();
}

/// Throughput report (`cargo test --release -p bicraft-xpress9 --test vectors -- --ignored bench`).
#[test]
#[ignore]
fn bench_throughput() {
    let (desktop, desktop_flags) = EncoderParams::power_bi_desktop();
    for (name, input) in [
        ("struct 4 MiB", generate("struct", 4 << 20, 1)),
        ("mixed 4 MiB", generate("mixed", 4 << 20, 2)),
    ] {
        for (label, params, flags) in [
            ("default (depth 9)", EncoderParams::default(), 0),
            ("desktop preset", desktop, desktop_flags),
        ] {
            let t = std::time::Instant::now();
            let mut enc = Encoder::with_flags(22, flags).unwrap();
            enc.start_session(&params, 1).unwrap();
            let c = enc.compress(&input, true).unwrap();
            let enc_s = t.elapsed().as_secs_f64();
            let t = std::time::Instant::now();
            let reps = 5;
            for _ in 0..reps {
                assert_eq!(decompress(&c, input.len()).unwrap().len(), input.len());
            }
            let dec_s = t.elapsed().as_secs_f64() / reps as f64;
            let mb = input.len() as f64 / 1e6;
            eprintln!(
                "{name}, {label}: ratio {:.3}, encode {:.1} MB/s, decode {:.1} MB/s",
                c.len() as f64 / input.len() as f64,
                mb / enc_s,
                mb / dec_s
            );
        }
    }
}
