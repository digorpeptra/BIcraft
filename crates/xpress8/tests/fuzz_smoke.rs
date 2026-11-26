//! Stable-Rust fuzz smoke test of the Plain LZ77 decoder (docs/fuzzing.md): random and mutated
//! inputs never panic, and every output stays within the caps.

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn random_inputs_never_panic() {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    for len in 0..600 {
        let data: Vec<u8> = (0..len).map(|_| (xorshift(&mut state) >> 24) as u8).collect();
        bicraft_xpress8::fuzz(&data);
    }
}

#[test]
fn mutated_valid_streams_never_panic() {
    let text: Vec<u8> = (0..4000u32)
        .flat_map(|i| format!("{};{}|", i % 31, i % 7).into_bytes())
        .collect();
    let good = bicraft_xpress8::compress_chunked(&text, 4096);
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    for _ in 0..2000 {
        let mut bytes = good.clone();
        for _ in 0..1 + xorshift(&mut state) % 4 {
            let at = (xorshift(&mut state) as usize) % bytes.len();
            bytes[at] = (xorshift(&mut state) >> 40) as u8;
        }
        let cut = (xorshift(&mut state) as usize) % (bytes.len() + 1);
        for input in [&bytes[..], &bytes[..cut]] {
            if let Ok(out) = bicraft_xpress8::decompress_chunked(input, None, 1 << 20) {
                assert!(out.len() <= 1 << 20);
            }
            let _ = bicraft_xpress8::decompress_chunked(input, Some(text.len() as u64), 1 << 20);
            bicraft_xpress8::fuzz(input);
        }
    }
}
