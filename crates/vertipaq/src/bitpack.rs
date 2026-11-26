//! `XMRENoSplit<N>` bit packing ([MS-XLDM] §2.7.1).
//!
//! Each little-endian `u64` word holds `64 / N` values of `N` bits, the first value in the
//! lowest bits. Values never straddle two words; the unused high bits of a word are zero.

use crate::compression::BIT_WIDTHS;

/// Values held by one 64-bit word at bit width `width` (0 for an invalid width).
pub fn values_per_word(width: u8) -> usize {
    if BIT_WIDTHS.contains(&width) {
        64 / usize::from(width)
    } else {
        0
    }
}

/// Number of `u64` words needed to pack `count` values at `width` bits.
pub fn words_needed(count: usize, width: u8) -> Option<usize> {
    let per = values_per_word(width);
    if per == 0 {
        return None;
    }
    Some(count.div_ceil(per))
}

/// Unpacks the first `count` values of `words` (raw bytes, whole words only).
///
/// Returns `None` when `width` is not an allowed width or the words hold fewer values.
pub fn unpack(words: &[u8], width: u8, count: usize) -> Option<Vec<u32>> {
    let per = values_per_word(width);
    if per == 0 {
        return None;
    }
    let available = (words.len() / 8).checked_mul(per)?;
    if count > available {
        return None;
    }
    // Allowed widths are at most 32, so the shift cannot overflow.
    let mask: u64 = (1u64 << width) - 1;
    let mut out = Vec::with_capacity(count);
    'outer: for w in crate::cursor::words(words) {
        let mut word = w;
        for _ in 0..per {
            if out.len() == count {
                break 'outer;
            }
            // `width` ≤ 32, so the masked value fits in u32.
            out.push((word & mask) as u32);
            word = word.checked_shr(u32::from(width)).unwrap_or(0);
        }
    }
    Some(out)
}

/// Packs `values` at `width` bits (inverse of [`unpack`]); `None` if a value does not fit.
pub fn pack(values: &[u32], width: u8) -> Option<Vec<u8>> {
    let per = values_per_word(width);
    if per == 0 {
        return None;
    }
    let limit = if width >= 32 {
        u64::from(u32::MAX)
    } else {
        (1u64 << width) - 1
    };
    let mut out = Vec::with_capacity(values.len().div_ceil(per) * 8);
    for chunk in values.chunks(per) {
        let mut word = 0u64;
        for (i, &v) in chunk.iter().enumerate() {
            if u64::from(v) > limit {
                return None;
            }
            word |= u64::from(v) << (i * usize::from(width));
        }
        out.extend_from_slice(&word.to_le_bytes());
    }
    Some(out)
}

/// Smallest allowed bit width that can hold `max_value`.
pub fn width_for(max_value: u32) -> u8 {
    let bits = (32 - max_value.leading_zeros()).max(1);
    BIT_WIDTHS
        .iter()
        .copied()
        .find(|&w| u32::from(w) >= bits)
        .unwrap_or(32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_word_counts() {
        let expected = [64, 32, 21, 16, 12, 10, 9, 8, 7, 6, 5, 4, 3, 2];
        for (w, e) in BIT_WIDTHS.iter().zip(expected) {
            assert_eq!(values_per_word(*w), e, "width {w}");
        }
        assert_eq!(values_per_word(11), 0);
        assert_eq!(values_per_word(0), 0);
    }

    #[test]
    fn known_layout_three_bits() {
        // Values 1..=5 at 3 bits: 0b101_100_011_010_001 in the low bits of word 0.
        let bytes = pack(&[1, 2, 3, 4, 5], 3).unwrap();
        assert_eq!(bytes.len(), 8);
        let w = u64::from_le_bytes(bytes[..8].try_into().unwrap());
        assert_eq!(w, 0b101_100_011_010_001);
        assert_eq!(unpack(&bytes, 3, 5).unwrap(), vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn values_do_not_straddle_words() {
        // 21 bits: 3 values per word, the 4th starts the next word.
        let vals = [0x1F_FFFF, 7, 0x10_0000, 9];
        let bytes = pack(&vals, 21).unwrap();
        assert_eq!(bytes.len(), 16);
        let w1 = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        assert_eq!(w1, 9);
        let w0 = u64::from_le_bytes(bytes[..8].try_into().unwrap());
        assert_eq!(w0 >> 63, 0, "top bit unused");
        assert_eq!(unpack(&bytes, 21, 4).unwrap(), vals.to_vec());
    }

    #[test]
    fn round_trip_every_width() {
        for w in BIT_WIDTHS {
            let max = if w == 32 { u32::MAX } else { (1u32 << w) - 1 };
            let vals: Vec<u32> = (0..200u32).map(|i| i.wrapping_mul(2_654_435_761) & max).collect();
            let bytes = pack(&vals, w).unwrap();
            assert_eq!(bytes.len() / 8, words_needed(vals.len(), w).unwrap());
            assert_eq!(unpack(&bytes, w, vals.len()).unwrap(), vals, "width {w}");
        }
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(unpack(&[0; 8], 11, 1), None);
        assert_eq!(unpack(&[0; 8], 32, 3), None);
        assert_eq!(pack(&[8], 3), None);
        assert_eq!(width_for(0), 1);
        assert_eq!(width_for(1023), 10);
        assert_eq!(width_for(1024), 12);
        assert_eq!(width_for(70_000), 21);
        assert_eq!(width_for(u32::MAX), 32);
    }
}
