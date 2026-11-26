//! Length-limited Huffman code construction for compressed string pages ([MS-XLDM] §2.7.4).

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::huffman::{MAX_CODE_LEN, MIN_CODE_LEN};

/// Codeword lengths (0 = unused, else `1..=15`) for byte frequencies `freqs`.
///
/// A plain Huffman code is built; while its longest codeword exceeds 15 bits the
/// frequencies are halved (keeping every used symbol at least 1) and the code rebuilt.
/// These are the lengths the engine writes (121 pages of engine-written dictionaries
/// compared, 1-bit codes included).
pub fn limited_code_lengths(freqs: &[u64; 256]) -> [u8; 256] {
    let mut weights = *freqs;
    loop {
        let lengths = huffman_lengths(&weights);
        if lengths.iter().all(|&l| l <= MAX_CODE_LEN) {
            return lengths.map(|l| if l == 0 { 0 } else { l.max(MIN_CODE_LEN) });
        }
        for w in weights.iter_mut() {
            if *w != 0 {
                *w = (*w >> 1).max(1);
            }
        }
    }
}

/// Unlimited Huffman code lengths (a lone symbol gets length 1).
fn huffman_lengths(weights: &[u64; 256]) -> [u8; 256] {
    let mut lengths = [0u8; 256];
    // Nodes: leaves 0..256 (only used ones are pushed), internal nodes after.
    let mut parent: Vec<usize> = vec![usize::MAX; 256];
    let mut heap = BinaryHeap::new();
    for (sym, &w) in weights.iter().enumerate() {
        if w != 0 {
            heap.push(Reverse((w, sym)));
        }
    }
    let used: Vec<usize> = heap.iter().map(|Reverse((_, s))| *s).collect();
    if used.len() == 1 {
        if let Some(&s) = used.first()
            && let Some(l) = lengths.get_mut(s)
        {
            *l = 1;
        }
        return lengths;
    }
    while heap.len() > 1 {
        let (Some(Reverse((wa, a))), Some(Reverse((wb, b)))) = (heap.pop(), heap.pop()) else {
            break;
        };
        let node = parent.len();
        parent.push(usize::MAX);
        if let Some(p) = parent.get_mut(a) {
            *p = node;
        }
        if let Some(p) = parent.get_mut(b) {
            *p = node;
        }
        heap.push(Reverse((wa.saturating_add(wb), node)));
    }
    for s in used {
        let mut depth = 0u32;
        let mut n = s;
        while let Some(&p) = parent.get(n) {
            if p == usize::MAX {
                break;
            }
            depth += 1;
            n = p;
        }
        if let Some(l) = lengths.get_mut(s) {
            *l = u8::try_from(depth).unwrap_or(u8::MAX);
        }
    }
    lengths
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::huffman::HuffmanCode;

    #[test]
    fn lengths_are_valid_and_limited() {
        // Fibonacci weights produce a maximally skewed tree (depth 25 unlimited).
        let mut freqs = [0u64; 256];
        let (mut a, mut b) = (1u64, 1u64);
        for f in freqs.iter_mut().take(26) {
            *f = a;
            (a, b) = (b, a + b);
        }
        let lengths = limited_code_lengths(&freqs);
        assert!(lengths.iter().take(26).all(|&l| (1..=15).contains(&l)));
        assert!(lengths.iter().skip(26).all(|&l| l == 0));
        HuffmanCode::from_lengths(&lengths).unwrap();

        let mut one = [0u64; 256];
        one[65] = 10;
        let l = limited_code_lengths(&one);
        assert_eq!(l[65], 1);
        HuffmanCode::from_lengths(&l).unwrap();

        let all = [1u64; 256];
        let l = limited_code_lengths(&all);
        assert!(l.iter().all(|&x| x == 8));
    }
}
