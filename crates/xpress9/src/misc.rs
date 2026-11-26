// Ported from Microsoft XPress9 (MIT), dev/ese/src/_xpress9/Xpress9Misc.c@7030fe7407615160e54d152e4ef704eede2fdd7e
// and the bitstream / constant definitions of dev/ese/src/_xpress9/Xpress9Internal.h@7030fe7407615160e54d152e4ef704eede2fdd7e
// Copyright (c) Microsoft Corporation. Licensed under the MIT License.
// See LICENSE-MIT-microsoft-xpress9 in this crate and THIRD_PARTY_NOTICES.md at the repository root.

//! Shared constants, the header CRC (`Xpress9Crc32`), `HuffmanReverseMask`, `GET_MSB`, and the
//! LSB-first bit reader/writer that replaces the `BIORD_*` / `BIOWR_*` macros.
//!
//! The upstream bit I/O works on a shift register of native machine words; the resulting byte
//! stream is a plain least-significant-bit-first bit stream, which is what the reader and writer
//! below produce and consume.

use crate::Error;

// ---------------------------------------------------------------- constants (Xpress9Internal.h)

/// `XPRESS9_MAGIC`: first word of every block header.
pub const XPRESS9_MAGIC: u32 = 0x4e86_d72a;

pub(crate) const HUFFMAN_CODEWORD_LENGTH_LIMIT: usize = 64;
pub(crate) const HUFFMAN_CODEWORD_LENGTH_BITS: u32 = 5;
pub(crate) const HUFFMAN_MAX_CODEWORD_LENGTH: usize = 27;

pub(crate) const HUFFMAN_ENCODED_TABLE_FILL: usize = HUFFMAN_MAX_CODEWORD_LENGTH + 1;
pub(crate) const HUFFMAN_ENCODED_TABLE_ZERO_REPT: usize = HUFFMAN_MAX_CODEWORD_LENGTH + 2;
pub(crate) const HUFFMAN_ENCODED_TABLE_PREV: usize = HUFFMAN_MAX_CODEWORD_LENGTH + 3;
pub(crate) const HUFFMAN_ENCODED_TABLE_ROW_0: usize = HUFFMAN_MAX_CODEWORD_LENGTH + 4;
pub(crate) const HUFFMAN_ENCODED_TABLE_ROW_1: usize = HUFFMAN_MAX_CODEWORD_LENGTH + 5;
pub(crate) const HUFFMAN_ENCODED_TABLE_SIZE: usize = HUFFMAN_MAX_CODEWORD_LENGTH + 6;
pub(crate) const HUFFMAN_ENCODED_TABLE_ZERO_REPT_MIN_COUNT: usize = 5;

pub(crate) const LZ77_MAX_MTF: usize = 4;
pub(crate) const LZ77_MAX_SHORT_LENGTH_LOG: u32 = 4;
pub(crate) const LZ77_MAX_SHORT_LENGTH: usize = 1 << LZ77_MAX_SHORT_LENGTH_LOG;
pub(crate) const LZ77_MAX_WINDOW_SIZE_LOG: usize = 24;
pub(crate) const LZ77_LONG_LENGTH_ALPHABET_SIZE_LOG: u32 = 9;
pub(crate) const LZ77_LONG_LENGTH_ALPHABET_SIZE: usize = 1 << (LZ77_LONG_LENGTH_ALPHABET_SIZE_LOG - 1);
pub(crate) const LZ77_MAX_LONG_LENGTH: usize = LZ77_LONG_LENGTH_ALPHABET_SIZE - LZ77_MAX_WINDOW_SIZE_LOG;
pub(crate) const LZ77_SHORT_SYMBOL_ALPHABET_SIZE: usize =
    256 + ((LZ77_MAX_WINDOW_SIZE_LOG + LZ77_MAX_MTF) << LZ77_MAX_SHORT_LENGTH_LOG);

// ------------------------------------------------------------------------------------- GET_MSB

/// `GET_MSB` (`_BitScanReverse` on the value truncated to 32 bits). Upstream leaves the output
/// undefined for 0; here 0 is an error.
pub(crate) fn get_msb(value: u64) -> Result<u32, Error> {
    let v = value as u32;
    if v == 0 {
        return Err(Error::Internal("GET_MSB of zero"));
    }
    Ok(31 - v.leading_zeros())
}

// --------------------------------------------------------------------------- HuffmanReverseMask

const S_HUFFMAN_BIT_REVERSE_TABLE_LOG: u32 = 6;

/// `s_uHuffmanBitReverseTable`: the 6-bit reversal of each index.
const S_HUFFMAN_BIT_REVERSE_TABLE: [u8; 64] = {
    let mut t = [0u8; 64];
    let mut i = 0;
    while i < 64 {
        let mut r = 0u8;
        let mut b = 0;
        while b < 6 {
            if (i >> b) & 1 != 0 {
                r |= 1 << (5 - b);
            }
            b += 1;
        }
        #[allow(clippy::indexing_slicing)] // const evaluation: an out-of-range index is a compile error
        {
            t[i] = r;
        }
        i += 1;
    }
    t
};

fn reverse6(index: u64) -> u64 {
    S_HUFFMAN_BIT_REVERSE_TABLE
        .get((index & ((1 << S_HUFFMAN_BIT_REVERSE_TABLE_LOG) - 1)) as usize)
        .copied()
        .unwrap_or(0) as u64
}

/// `HuffmanReverseMask`: reverse the `bits` low bits of `mask`, six bits at a time.
pub(crate) fn huffman_reverse_mask(mut mask: u64, mut bits: u32) -> u64 {
    let mut new_mask: u64 = 0;
    while bits >= S_HUFFMAN_BIT_REVERSE_TABLE_LOG {
        new_mask = (new_mask << S_HUFFMAN_BIT_REVERSE_TABLE_LOG).wrapping_add(reverse6(mask));
        mask >>= S_HUFFMAN_BIT_REVERSE_TABLE_LOG;
        bits -= S_HUFFMAN_BIT_REVERSE_TABLE_LOG;
    }
    if bits != 0 {
        new_mask = (new_mask << S_HUFFMAN_BIT_REVERSE_TABLE_LOG).wrapping_add(reverse6(mask));
        new_mask >>= S_HUFFMAN_BIT_REVERSE_TABLE_LOG - bits;
    }
    new_mask
}

// --------------------------------------------------------------------------------- Xpress9Crc32

/// Generating polynomial of the header checksum (CRC-32C, Castagnoli, reflected).
const CRC32_POLY: u32 = 0x82f6_3b78;

/// `Xpress9Crc32Table`, generated rather than copied (identical values; see the unit test).
static XPRESS9_CRC32_TABLE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { (c >> 1) ^ CRC32_POLY } else { c >> 1 };
            k += 1;
        }
        #[allow(clippy::indexing_slicing)] // const evaluation: an out-of-range index is a compile error
        {
            t[i] = c;
        }
        i += 1;
    }
    t
};

/// `Xpress9Crc32 (pData, uSize, uCrc)`.
pub fn xpress9_crc32(data: &[u8], crc: u32) -> u32 {
    let mut crc = crc ^ 0xffff_ffff;
    for &b in data {
        let entry = XPRESS9_CRC32_TABLE
            .get(((crc ^ b as u32) & 255) as usize)
            .copied()
            .unwrap_or(0);
        crc = (crc >> 8) ^ entry;
    }
    crc ^ 0xffff_ffff
}

// ------------------------------------------------------------------------------- bitstream I/O

/// LSB-first bit reader over one block's bitstream (`BIORD_*`). Reads past the end yield zero
/// bits; the caller bounds consumption against the block's encoded size.
pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    pos: u64,
}

impl<'a> BitReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        BitReader { data, pos: 0 }
    }

    /// Number of bits consumed so far (`BIORD_TELL_CONSUMED_BITS_STATE`).
    #[inline]
    pub(crate) fn consumed_bits(&self) -> u64 {
        self.pos
    }

    #[inline]
    fn word_at(&self, byte: usize) -> u64 {
        if let Some(w) = self.data.get(byte..).and_then(|s| s.first_chunk::<8>()) {
            return u64::from_le_bytes(*w);
        }
        let mut w = [0u8; 8];
        for (k, slot) in w.iter_mut().enumerate() {
            *slot = byte
                .checked_add(k)
                .and_then(|i| self.data.get(i))
                .copied()
                .unwrap_or(0);
        }
        u64::from_le_bytes(w)
    }

    /// `BIORD_LOOKUP_FIXED (n)`: the next `n <= 32` bits without consuming them.
    #[inline]
    pub(crate) fn peek(&self, n: u32) -> u32 {
        let byte = usize::try_from(self.pos >> 3).unwrap_or(usize::MAX);
        let shift = (self.pos & 7) as u32;
        let word = self.word_at(byte) >> shift;
        (word & ((1u64 << n.min(32)) - 1)) as u32
    }

    /// `BIORD_SKIP (n)`.
    #[inline]
    pub(crate) fn skip(&mut self, n: u32) {
        self.pos = self.pos.saturating_add(u64::from(n));
    }

    /// `BIORD_LOAD (value, n)`.
    #[inline]
    pub(crate) fn load(&mut self, n: u32) -> u32 {
        let v = self.peek(n);
        self.skip(n);
        v
    }
}

/// LSB-first bit writer (`BIOWR_*`) appending to a byte vector.
#[derive(Default)]
pub(crate) struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    nbits: u32,
}

/// A saved writer position, used to undo a trial encoding (`BIO_STATE` copies upstream).
#[derive(Clone, Copy)]
pub(crate) struct BitWriterMark {
    len: usize,
    acc: u64,
    nbits: u32,
}

impl BitWriter {
    pub(crate) fn new() -> Self {
        BitWriter::default()
    }

    /// `BIOWR (value, n)` for `n <= 32`.
    #[inline]
    pub(crate) fn write(&mut self, value: u64, n: u32) {
        // Upstream adds (rather than ORs) the value into the shift register.
        self.acc = self.acc.wrapping_add(value << self.nbits);
        self.nbits += n;
        while self.nbits >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.nbits -= 8;
        }
    }

    /// `HUFFMAN_WRITE_SYMBOL (packed)`: packed = (codeword << 5) | length.
    #[inline]
    pub(crate) fn write_symbol(&mut self, packed: u32) {
        let len = packed & ((1 << HUFFMAN_CODEWORD_LENGTH_BITS) - 1);
        self.write(u64::from(packed >> HUFFMAN_CODEWORD_LENGTH_BITS), len);
    }

    /// Total number of bits written so far.
    #[inline]
    pub(crate) fn bit_position(&self) -> u64 {
        (self.out.len() as u64) * 8 + u64::from(self.nbits)
    }

    pub(crate) fn mark(&self) -> BitWriterMark {
        BitWriterMark {
            len: self.out.len(),
            acc: self.acc,
            nbits: self.nbits,
        }
    }

    pub(crate) fn restore(&mut self, mark: BitWriterMark) {
        self.out.truncate(mark.len);
        self.acc = mark.acc;
        self.nbits = mark.nbits;
    }

    /// `BIOWR_FLUSH ()`, then hand out the bytes.
    pub(crate) fn finish(mut self) -> Vec<u8> {
        if self.nbits != 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }

    /// Overwrite a little-endian u32 at a byte offset already written (the block header).
    pub(crate) fn patch_u32(bytes: &mut [u8], word_index: usize, value: u32) -> Result<(), Error> {
        let start = word_index
            .checked_mul(4)
            .ok_or(Error::Internal("header word index"))?;
        let slot = bytes
            .get_mut(start..start.saturating_add(4))
            .ok_or(Error::Internal("header word out of range"))?;
        for (dst, src) in slot.iter_mut().zip(value.to_le_bytes()) {
            *dst = src;
        }
        Ok(())
    }
}

/// Read the `index`-th little-endian u32 of a byte slice.
pub(crate) fn read_u32(bytes: &[u8], index: usize) -> Option<u32> {
    let start = index.checked_mul(4)?;
    bytes
        .get(start..)?
        .first_chunk::<4>()
        .map(|w| u32::from_le_bytes(*w))
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn crc_table_matches_upstream_entries() {
        // Values from Xpress9Misc.c (and docs/formats/xpress9.md section 5).
        assert_eq!(XPRESS9_CRC32_TABLE[1], 0xf26b_8303);
        assert_eq!(XPRESS9_CRC32_TABLE[2], 0xe13b_70f7);
        assert_eq!(XPRESS9_CRC32_TABLE[128], 0x82f6_3b78);
        assert_eq!(XPRESS9_CRC32_TABLE[255], 0xad7d_5351);
    }

    #[test]
    fn crc32c_check_values() {
        assert_eq!(xpress9_crc32(b"123456789", 0), 0xe306_9283);
        let mut header = [0u8; 28];
        header[..4].copy_from_slice(&XPRESS9_MAGIC.to_le_bytes());
        assert_eq!(xpress9_crc32(&header, 0), 0xc980_6d35);
    }

    #[test]
    fn reverse_mask() {
        assert_eq!(huffman_reverse_mask(0b1, 1), 0b1);
        assert_eq!(huffman_reverse_mask(0b011, 3), 0b110);
        assert_eq!(huffman_reverse_mask(0b1_000000_000001, 13), 0b1_000000_000001);
        assert_eq!(huffman_reverse_mask(0b0_000000_000011, 13), 0b1_100000_000000);
        for bits in 1..=27u32 {
            for v in [0u64, 1, 5, (1 << bits) - 1, (1 << bits) / 3] {
                let expected = (v.reverse_bits() >> (64 - bits)) & ((1 << bits) - 1);
                assert_eq!(huffman_reverse_mask(v, bits), expected, "v={v} bits={bits}");
            }
        }
    }

    #[test]
    fn bit_round_trip() {
        let mut w = BitWriter::new();
        w.write(0b101, 3);
        w.write(0x1234_5678, 32);
        w.write(1, 1);
        w.write(0x7f_ffff, 23);
        let bits = w.bit_position();
        assert_eq!(bits, 59);
        let bytes = w.finish();
        let mut r = BitReader::new(&bytes);
        assert_eq!(r.load(3), 0b101);
        assert_eq!(r.load(32), 0x1234_5678);
        assert_eq!(r.load(1), 1);
        assert_eq!(r.load(23), 0x7f_ffff);
        assert_eq!(r.load(16), 0);
        assert_eq!(r.consumed_bits(), 75);
    }
}
