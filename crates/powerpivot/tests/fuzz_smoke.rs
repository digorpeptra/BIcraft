//! Stable-Rust fuzz smoke test of the Power Pivot reader (docs/fuzzing.md): mutations of a
//! synthetic backup and workbook never panic, with small caps.

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn mutate(good: &[u8], state: &mut u64, rounds: usize) {
    for _ in 0..rounds {
        let mut bytes = good.to_vec();
        for _ in 0..1 + xorshift(state) % 8 {
            let at = (xorshift(state) as usize) % bytes.len();
            bytes[at] = (xorshift(state) >> 40) as u8;
        }
        let cut = (xorshift(state) as usize) % (bytes.len() + 1);
        bicraft_powerpivot::fuzz(&bytes);
        bicraft_powerpivot::fuzz(&bytes[..cut]);
    }
}

#[test]
fn mutated_backups_never_panic() {
    let model = bicraft_powerpivot::synthetic::sample_model();
    let item = bicraft_powerpivot::synthetic::backup(&model).unwrap();
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    mutate(&item, &mut state, 300);
}

#[test]
fn mutated_workbooks_never_panic() {
    let wb = bicraft_powerpivot::synthetic::sample().unwrap();
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    mutate(&wb, &mut state, 150);
    for len in 0..300 {
        let data: Vec<u8> = (0..len).map(|_| (xorshift(&mut state) >> 24) as u8).collect();
        bicraft_powerpivot::fuzz(&data);
    }
}
