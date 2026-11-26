//! `compress_datamodel`: the single-threaded `DataModel` framing writer.
#![allow(clippy::indexing_slicing)]

mod common;

use bicraft_xpress9::datamodel::{
    BANNER_SIZE, DEFAULT_CHUNK_SIZE, MAX_CHUNK_SIZE, MultiThreadedHeader, SINGLE_THREADED_BANNER_TEXT,
    compress_with_plan, single_threaded_banner,
};
use bicraft_xpress9::{
    CompressionMode, Encoder, EncoderParams, Error, FramingPlan, Layout, compress_datamodel,
    decompress_datamodel, inspect_datamodel,
};
use common::{generate, manifest, read_vector, synthetic_banner};

fn check_round_trip(part: &[u8], input: &[u8]) {
    assert_eq!(decompress_datamodel(part, input.len()).unwrap(), input);
}

#[test]
fn banner_constant_fills_the_banner_field() {
    let banner = single_threaded_banner();
    assert_eq!(banner.len(), BANNER_SIZE);
    assert_eq!(SINGLE_THREADED_BANNER_TEXT.encode_utf16().count(), 50);
    assert_eq!(&banner[100..], &[0, 0], "one UTF-16 NUL after the 50 characters");
    assert_eq!(
        bicraft_xpress9::datamodel::detect_layout(&banner).unwrap(),
        Layout::SingleThreaded
    );
}

#[test]
fn default_plan_is_the_desktop_preset() {
    let plan = FramingPlan::new(None);
    let (params, flags) = EncoderParams::power_bi_desktop();
    assert_eq!(plan.params, params);
    assert_eq!(plan.encoder_flags, flags);
    assert_eq!(plan.chunk_size, DEFAULT_CHUNK_SIZE);
    assert_eq!(plan.banner, single_threaded_banner());
}

#[test]
fn small_inputs_round_trip() {
    for input in [
        Vec::new(),
        vec![42u8],
        generate("text", 1000, 7),
        generate("mixed", 150_000, 8),
    ] {
        let part = compress_datamodel(&input, None).unwrap();
        assert_eq!(&part[..BANNER_SIZE], &single_threaded_banner()[..]);
        let info = inspect_datamodel(&part).unwrap();
        assert_eq!(info.layout, Layout::SingleThreaded);
        assert_eq!(info.total_uncompressed, input.len() as u64);
        assert_eq!(info.chunks.len(), input.len().div_ceil(DEFAULT_CHUNK_SIZE));
        check_round_trip(&part, &input);
        // deterministic for a given input
        assert_eq!(compress_datamodel(&input, None).unwrap(), part);
    }
}

#[test]
fn chunks_share_one_session() {
    let input = generate("mixed", 300_000, 11);
    let plan = FramingPlan {
        chunk_size: 64 << 10,
        ..FramingPlan::new(None)
    };
    let part = compress_with_plan(&input, &plan).unwrap();
    let info = inspect_datamodel(&part).unwrap();
    assert_eq!(info.sessions, 1);
    assert_eq!(info.chunks.len(), 5);
    let sizes: Vec<u32> = info.chunks.iter().map(|c| c.uncompressed_size).collect();
    assert_eq!(sizes, [65536, 65536, 65536, 65536, 300_000 - 4 * 65536]);
    let blocks: Vec<_> = info.chunks.iter().flat_map(|c| c.blocks.iter()).collect();
    let indices: Vec<u32> = blocks.iter().map(|b| b.block_index).collect();
    assert_eq!(indices, (0..indices.len() as u32).collect::<Vec<_>>());
    assert!(
        blocks
            .iter()
            .all(|b| b.session_signature == blocks[0].session_signature)
    );
    check_round_trip(&part, &input);
}

#[test]
fn plan_follows_a_sample() {
    // a sample written with unusual settings and a non-default chunk size
    let input = generate("struct", 200_000, 12);
    let params = EncoderParams {
        max_stream_length: 0,
        mtf_entry_count: 2,
        lookup_depth: 2,
        window_size_log2: 18,
        optimization_level: 0,
        ptr_min_match_length: 3,
        mtf_min_match_length: 3,
    };
    let sample_plan = FramingPlan {
        banner: synthetic_banner(false),
        params,
        encoder_flags: 0,
        chunk_size: 96 << 10,
        ..FramingPlan::new(None)
    };
    let sample = compress_with_plan(&input, &sample_plan).unwrap();
    let info = inspect_datamodel(&sample).unwrap();
    assert_eq!(info.banner, synthetic_banner(false));

    let plan = FramingPlan::new(Some(&info));
    assert_eq!(
        plan.banner,
        synthetic_banner(false),
        "a single-threaded banner is copied"
    );
    assert_eq!(plan.chunk_size, 96 << 10);
    assert_eq!(plan.params.window_size_log2, 18);
    assert_eq!(plan.params.mtf_entry_count, 2);
    assert_eq!(plan.params.ptr_min_match_length, 3);
    assert_eq!(plan.params.mtf_min_match_length, 3);

    let other = generate("text", 250_000, 13);
    let part = compress_datamodel(&other, Some(&info)).unwrap();
    let out = inspect_datamodel(&part).unwrap();
    assert_eq!(out.banner, info.banner);
    assert_eq!(out.chunks.len(), other.len().div_ceil(96 << 10));
    assert_eq!(
        out.chunks[0].blocks[0].flags & !0x1fff,
        info.chunks[0].blocks[0].flags & !0x1fff,
        "same window, MTF and minimum-match flags"
    );
    check_round_trip(&part, &other);
}

#[test]
fn multithreaded_or_bare_samples_still_give_the_single_threaded_layout() {
    let input = generate("text", 70_000, 14);
    let multi = manifest().into_iter().find(|v| v.kind == "dm_multi").unwrap();
    let info = inspect_datamodel(&read_vector(&multi)).unwrap();
    assert_eq!(info.layout, Layout::MultiThreaded);
    let plan = FramingPlan::new(Some(&info));
    assert_eq!(plan.banner, single_threaded_banner());
    // 32 KiB in the sample is outside the accepted range
    assert_eq!(plan.chunk_size, DEFAULT_CHUNK_SIZE);
    let part = compress_datamodel(&input, Some(&info)).unwrap();
    assert_eq!(inspect_datamodel(&part).unwrap().layout, Layout::SingleThreaded);
    check_round_trip(&part, &input);

    let mut abf = vec![0xff, 0xfe];
    for u in "STREAM_STORAGE_SIGNATURE_)!@#$%^&*(".encode_utf16() {
        abf.extend(u.to_le_bytes());
    }
    abf.resize(4096, 0);
    let bare = inspect_datamodel(&abf).unwrap();
    assert!(bare.banner.is_empty());
    let part = compress_datamodel(&abf, Some(&bare)).unwrap();
    assert_eq!(&part[..BANNER_SIZE], &single_threaded_banner()[..]);
    check_round_trip(&part, &abf);
}

#[test]
fn bad_plans_are_errors() {
    let input = [1u8, 2, 3];
    let mut plan = FramingPlan::new(None);
    plan.banner.pop();
    assert!(matches!(
        compress_with_plan(&input, &plan),
        Err(Error::BadArgument(_))
    ));
    let mut plan = FramingPlan::new(None);
    plan.chunk_size = 0;
    assert!(matches!(
        compress_with_plan(&input, &plan),
        Err(Error::BadArgument(_))
    ));
    let mut plan = FramingPlan::new(None);
    plan.params.window_size_log2 = 30;
    assert!(compress_with_plan(&input, &plan).is_err());
}

/// F3 (docs/fuzzing.md): the sizes the chunk envelopes declare are checked against the limit
/// before anything is decoded, so a small part claiming a huge backup fails at once.
#[test]
fn declared_sizes_are_checked_before_decoding() {
    let input = vec![0u8; 1 << 20];
    let part = compress_datamodel(&input, None).unwrap();
    assert!(part.len() < 4096, "zeros compress well: {}", part.len());
    assert_eq!(decompress_datamodel(&part, 1 << 20).unwrap(), input);
    assert_eq!(
        decompress_datamodel(&part, (1 << 20) - 1),
        Err(Error::LimitExceeded((1 << 20) - 1))
    );
    // Inflate the first chunk's declared size: refused from the envelope, not after decoding.
    let mut lying = part.clone();
    lying[BANNER_SIZE..BANNER_SIZE + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        inspect_datamodel(&lying).unwrap().total_uncompressed,
        u64::from(u32::MAX)
    );
    assert_eq!(
        decompress_datamodel(&lying, 1 << 30),
        Err(Error::LimitExceeded(1 << 30))
    );
}

/// The part with each block's session signature (word 5) and header CRC (word 7) zeroed.
fn mask_signatures(part: &[u8]) -> Vec<u8> {
    let info = inspect_datamodel(part).unwrap();
    let mut v = part.to_vec();
    for c in &info.chunks {
        let mut at = c.offset + 8;
        for b in &c.blocks {
            v[at + 20..at + 24].fill(0);
            v[at + 28..at + 32].fill(0);
            at += b.block_size();
        }
    }
    v
}

/// Checks the single-session invariants: one signature, block indices 0, 1, 2, ...
fn assert_one_session(part: &[u8]) {
    let info = inspect_datamodel(part).unwrap();
    assert_eq!(info.layout, Layout::SingleThreaded);
    let blocks: Vec<_> = info.chunks.iter().flat_map(|c| c.blocks.iter()).collect();
    let indices: Vec<u32> = blocks.iter().map(|b| b.block_index).collect();
    assert_eq!(indices, (0..indices.len() as u32).collect::<Vec<_>>());
    assert!(
        blocks
            .iter()
            .all(|b| b.session_signature == blocks[0].session_signature)
    );
}

#[test]
fn default_mode_is_fast() {
    assert_eq!(FramingPlan::new(None).mode, CompressionMode::Fast);
    assert_eq!(FramingPlan::new(None).threads, 0);
    let input = generate("mixed", 150_000, 21);
    let plan = FramingPlan {
        chunk_size: 32 << 10,
        ..FramingPlan::new(None)
    };
    assert_eq!(
        compress_with_plan(&input, &plan).unwrap(),
        compress_with_plan(&input, &plan.clone().with_mode(CompressionMode::Fast)).unwrap()
    );
}

/// The exact mode is the C encoder's stream: the `dm_single` vector (written by the upstream C
/// code, one session across chunks) comes back byte for byte apart from the signatures.
#[test]
fn exact_mode_matches_the_c_oracle() {
    let v = manifest().into_iter().find(|v| v.kind == "dm_single").unwrap();
    let input = generate(&v.gen_name, v.size, v.seed);
    let plan = FramingPlan {
        banner: synthetic_banner(false),
        params: v.params,
        encoder_flags: 0,
        chunk_size: v.chunk,
        mode: CompressionMode::Exact,
        threads: 0,
    };
    let ours = compress_with_plan(&input, &plan).unwrap();
    let expected = read_vector(&v);
    assert_eq!(ours.len(), expected.len());
    assert!(mask_signatures(&ours) == mask_signatures(&expected));
}

#[test]
fn fast_mode_does_not_depend_on_the_thread_count() {
    let input = generate("mixed", 700_000, 22);
    for mode in [CompressionMode::Balanced, CompressionMode::Fast] {
        let plan = FramingPlan {
            chunk_size: 64 << 10,
            ..FramingPlan::new(None).with_mode(mode)
        };
        let reference = compress_with_plan(
            &input,
            &FramingPlan {
                threads: 1,
                ..plan.clone()
            },
        )
        .unwrap();
        for threads in [0, 2, 3, 8, 64] {
            let part = compress_with_plan(
                &input,
                &FramingPlan {
                    threads,
                    ..plan.clone()
                },
            )
            .unwrap();
            assert!(part == reference, "{mode:?} with {threads} threads");
        }
        assert_eq!(inspect_datamodel(&reference).unwrap().chunks.len(), 11);
        assert_one_session(&reference);
        check_round_trip(&reference, &input);
    }
}

#[test]
fn modes_agree_on_a_single_chunk() {
    // one chunk, no history: the balanced mode runs the very encoder of the exact mode
    let input = generate("struct", 300_000, 23);
    let plan = FramingPlan::new(None);
    let exact = compress_with_plan(&input, &plan.clone().with_mode(CompressionMode::Exact)).unwrap();
    let balanced = compress_with_plan(&input, &plan.clone().with_mode(CompressionMode::Balanced)).unwrap();
    assert_eq!(balanced, exact);
    // the fast mode searches less deeply: a different (valid) stream
    let fast = compress_datamodel(&input, None).unwrap();
    assert_ne!(fast, exact);
    check_round_trip(&fast, &input);
}

#[test]
fn fast_chunks_reach_back_into_earlier_chunks() {
    // four copies of 64 KiB of noise: every chunk after the first is one long match into the
    // chunk before it, which only the primed history makes reachable (the shallow search of
    // `Fast` does not find matches that far back in noise)
    let block = generate("random", 64 << 10, 24);
    let input = block.repeat(4);
    for mode in [CompressionMode::Exact, CompressionMode::Balanced] {
        let plan = FramingPlan {
            chunk_size: 64 << 10,
            ..FramingPlan::new(None).with_mode(mode)
        };
        let part = compress_with_plan(&input, &plan).unwrap();
        assert!(
            part.len() < block.len() + block.len() / 8,
            "{mode:?}: {} bytes",
            part.len()
        );
        assert_one_session(&part);
        check_round_trip(&part, &input);
    }
}

#[test]
fn fast_mode_round_trips_odd_shapes() {
    let inputs = [
        vec![9u8],
        generate("text", 5, 31),
        generate("mixed", (64 << 10) + 1, 32),
        generate("zeros", 300_000, 33),
        generate("random", 200_000, 34),
    ];
    for input in &inputs {
        for (chunk, window) in [(64 << 10, 16), (100_000, 18), (DEFAULT_CHUNK_SIZE, 22)] {
            let mut plan = FramingPlan::new(None);
            plan.chunk_size = chunk;
            plan.params.window_size_log2 = window;
            let part = compress_with_plan(input, &plan).unwrap();
            assert_one_session(&part);
            check_round_trip(&part, input);
        }
    }
}

#[test]
fn prime_must_follow_start_session() {
    let (params, flags) = EncoderParams::power_bi_desktop();
    let mut enc = Encoder::with_flags(22, flags).unwrap();
    assert!(enc.prime(b"history").is_err(), "no session yet");
    enc.start_session(&params, 1).unwrap();
    enc.compress(b"some input", false).unwrap();
    assert!(enc.prime(b"history").is_err(), "data already attached");
    enc.start_session(&params, 1).unwrap();
    enc.prime(b"history").unwrap();
}

/// G17 regression: Desktop 2.158's engine refuses a single-threaded part whose chunks hold more
/// than 2 MiB (2 MiB + 1 byte already fails to load), so the writer refuses such plans and never
/// takes a larger chunk size from a sample (`docs/formats/xpress9.md` §6).
#[test]
fn chunks_never_exceed_two_mebibytes() {
    let input = generate("text", 10_000, 15);
    let mut plan = FramingPlan::new(None);
    plan.chunk_size = MAX_CHUNK_SIZE;
    assert!(compress_with_plan(&input, &plan).is_ok());
    plan.chunk_size = MAX_CHUNK_SIZE + 1;
    assert!(matches!(
        compress_with_plan(&input, &plan),
        Err(Error::BadArgument(_))
    ));
    // a multithreaded sample with 4 MiB chunks: the writer keeps 2 MiB
    let info = bicraft_xpress9::DataModelInfo {
        layout: Layout::MultiThreaded,
        banner: Vec::new(),
        multithreaded_header: Some(MultiThreadedHeader {
            main_chunks_per_thread: 1,
            prefix_chunks_per_thread: 1,
            prefix_thread_count: 1,
            main_thread_count: 1,
            chunk_uncompressed_size: 4 << 20,
        }),
        chunks: Vec::new(),
        sessions: 0,
        total_uncompressed: 0,
        trailing_bytes: 0,
    };
    assert_eq!(FramingPlan::new(Some(&info)).chunk_size, DEFAULT_CHUNK_SIZE);
    // and the 2 MiB of Desktop's own samples are kept
    let mut two = info.clone();
    if let Some(h) = two.multithreaded_header.as_mut() {
        h.chunk_uncompressed_size = 2 << 20;
    }
    assert_eq!(FramingPlan::new(Some(&two)).chunk_size, 2 << 20);
}
