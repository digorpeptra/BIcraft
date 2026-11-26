//! Canonical Huffman code of compressed string-dictionary pages ([MS-XLDM] §2.7.4).
//!
//! * The alphabet is the 256 byte values. A page stores each symbol's codeword length in a
//!   128-byte array, two 4-bit lengths per byte: the low nibble is symbol `2i`, the high
//!   nibble symbol `2i + 1`. Length 0 means unused; used lengths are 1..=15 (the engine writes
//!   1-bit codes for a dominant symbol, e.g. `0` in `x00000`…`x01169`).
//! * Codes are canonical: symbols sorted by (length, symbol value) get consecutive codes,
//!   shifted left when the length grows (this reproduces the worked example of §2.7.4.2).
//! * The bit stream is read most-significant bit first within little-endian 16-bit words,
//!   i.e. bit `p` is bit `15 - p % 16` of word `p / 16`. Strings follow each other without
//!   padding; record handles give each string's starting bit.

/// Longest allowed codeword.
pub const MAX_CODE_LEN: u8 = 15;
/// Shortest allowed codeword.
pub const MIN_CODE_LEN: u8 = 1;

/// A canonical Huffman code over bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HuffmanCode {
    lengths: [u8; 256],
    /// Number of codes of each length (index = length).
    counts: [u16; 16],
    /// Symbols sorted by (length, value).
    symbols: Vec<u8>,
}

impl HuffmanCode {
    /// Builds the code from the 128-byte nibble-packed length array of a page.
    pub fn from_encode_array(encode_array: &[u8]) -> Result<HuffmanCode, &'static str> {
        if encode_array.len() != 128 {
            return Err("encode array must be 128 bytes");
        }
        let mut lengths = [0u8; 256];
        for (i, &b) in encode_array.iter().enumerate() {
            lengths[2 * i] = b & 0x0F;
            lengths[2 * i + 1] = b >> 4;
        }
        HuffmanCode::from_lengths(&lengths)
    }

    /// Builds the code from one codeword length per byte value (0 = unused).
    pub fn from_lengths(lengths: &[u8; 256]) -> Result<HuffmanCode, &'static str> {
        let mut counts = [0u16; 16];
        let mut kraft: u32 = 0;
        for &len in lengths {
            if len == 0 {
                continue;
            }
            if !(MIN_CODE_LEN..=MAX_CODE_LEN).contains(&len) {
                return Err("codeword length outside 1..=15");
            }
            counts[usize::from(len)] += 1;
            kraft += 1 << (MAX_CODE_LEN - len);
        }
        if kraft > 1 << MAX_CODE_LEN {
            return Err("codeword lengths are over-subscribed");
        }
        let mut symbols: Vec<u8> = (0..=255u8).filter(|&s| lengths[usize::from(s)] != 0).collect();
        symbols.sort_by_key(|&s| (lengths[usize::from(s)], s));
        Ok(HuffmanCode {
            lengths: *lengths,
            counts,
            symbols,
        })
    }

    /// Codeword length of each byte value.
    pub fn lengths(&self) -> &[u8; 256] {
        &self.lengths
    }

    /// The nibble-packed 128-byte form of the lengths (inverse of `from_encode_array`).
    pub fn encode_array(&self) -> [u8; 128] {
        let mut out = [0u8; 128];
        for (i, b) in out.iter_mut().enumerate() {
            *b = (self.lengths[2 * i] & 0x0F) | (self.lengths[2 * i + 1] << 4);
        }
        out
    }

    /// The canonical codeword of `symbol` as `(code, length)`, if the symbol is used.
    pub fn codeword(&self, symbol: u8) -> Option<(u32, u8)> {
        let len = self.lengths[usize::from(symbol)];
        if len == 0 {
            return None;
        }
        let mut code: u32 = 0;
        let mut prev_len = 0u8;
        for &s in &self.symbols {
            let l = self.lengths[usize::from(s)];
            if prev_len != 0 {
                code = (code + 1) << (l - prev_len);
            }
            prev_len = l;
            if s == symbol {
                return Some((code, l));
            }
        }
        None
    }

    /// Decodes the symbols between bit `start` (inclusive) and bit `end` (exclusive).
    pub fn decode(&self, stream: &[u8], start: u64, end: u64) -> Result<Vec<u8>, &'static str> {
        if end < start {
            return Err("string ends before it starts");
        }
        let mut out = Vec::with_capacity(usize::try_from((end - start) / 2).unwrap_or(0).min(1 << 20));
        let mut pos = start;
        while pos < end {
            let mut code: u32 = 0;
            let mut first: u32 = 0;
            let mut index: u32 = 0;
            let mut found = None;
            for len in 1..=usize::from(MAX_CODE_LEN) {
                if pos >= end {
                    return Err("codeword runs past the end of the string");
                }
                code |= u32::from(read_bit(stream, pos).ok_or("bit offset past the buffer")?);
                pos += 1;
                let count = u32::from(self.counts[len]);
                if code < first + count {
                    found = self.symbols.get((index + code - first) as usize).copied();
                    break;
                }
                index += count;
                first = (first + count) << 1;
                code <<= 1;
            }
            out.push(found.ok_or("invalid codeword")?);
        }
        Ok(out)
    }
}

/// Bit `pos` of a page bit stream (MSB first within little-endian 16-bit words).
pub fn read_bit(stream: &[u8], pos: u64) -> Option<bool> {
    let word = usize::try_from(pos / 16).ok()?;
    let in_word = (pos % 16) as u32;
    let byte_index = word
        .checked_mul(2)?
        .checked_add(if in_word < 8 { 1 } else { 0 })?;
    let byte = *stream.get(byte_index)?;
    Some((byte >> (7 - in_word % 8)) & 1 == 1)
}

/// Writes codewords in the page bit-stream layout (for tests and future writers).
#[derive(Debug, Default, Clone)]
pub struct BitWriter {
    bytes: Vec<u8>,
    bits: u64,
}

impl BitWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Current length in bits (the next string's bit offset).
    pub fn bit_len(&self) -> u64 {
        self.bits
    }

    /// Appends the low `len` bits of `code`, most significant first.
    pub fn push(&mut self, code: u32, len: u8) {
        for i in (0..len).rev() {
            let bit = (code >> i) & 1 == 1;
            let word = (self.bits / 16) as usize;
            let in_word = (self.bits % 16) as u32;
            let byte_index = word * 2 + if in_word < 8 { 1 } else { 0 };
            if self.bytes.len() < word * 2 + 2 {
                self.bytes.resize(word * 2 + 2, 0);
            }
            if bit && let Some(b) = self.bytes.get_mut(byte_index) {
                *b |= 1 << (7 - in_word % 8);
            }
            self.bits += 1;
        }
    }

    /// The written bytes, padded to whole 16-bit words.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The [MS-XLDM] §2.7.4.2 example: "Female" and "Male" with lengths
    /// e=2, l=2, F=3, M=3, a=3, m=3 encode to the 25 bits below.
    fn example_code() -> HuffmanCode {
        let mut lengths = [0u8; 256];
        for (sym, len) in [(b'F', 3), (b'M', 3), (b'a', 3), (b'e', 2), (b'l', 2), (b'm', 3)] {
            lengths[usize::from(sym)] = len;
        }
        HuffmanCode::from_lengths(&lengths).unwrap()
    }

    const EXAMPLE_BITS: &str = "1000011111001001011100100";

    fn bits_to_stream(bits: &str) -> Vec<u8> {
        let mut w = BitWriter::new();
        for c in bits.chars() {
            w.push(u32::from(c == '1'), 1);
        }
        w.into_bytes()
    }

    #[test]
    fn spec_example_codewords() {
        let code = example_code();
        assert_eq!(code.codeword(b'e'), Some((0b00, 2)));
        assert_eq!(code.codeword(b'l'), Some((0b01, 2)));
        assert_eq!(code.codeword(b'F'), Some((0b100, 3)));
        assert_eq!(code.codeword(b'M'), Some((0b101, 3)));
        assert_eq!(code.codeword(b'a'), Some((0b110, 3)));
        assert_eq!(code.codeword(b'm'), Some((0b111, 3)));
        assert_eq!(code.codeword(b'z'), None);
    }

    #[test]
    fn spec_example_decodes() {
        let code = example_code();
        let stream = bits_to_stream(EXAMPLE_BITS);
        assert_eq!(code.decode(&stream, 0, 25).unwrap(), b"FemaleMale");
        // "Female" is 15 bits, "Male" the next 10.
        assert_eq!(code.decode(&stream, 0, 15).unwrap(), b"Female");
        assert_eq!(code.decode(&stream, 15, 25).unwrap(), b"Male");
    }

    #[test]
    fn stream_layout_is_msb_first_in_le_words() {
        // The first 16 bits "1000011111001001" form the word 0x87C9, stored little-endian.
        let stream = bits_to_stream(EXAMPLE_BITS);
        assert_eq!(&stream[..2], &[0xC9, 0x87]);
        assert_eq!(stream.len(), 4);
        // Bits 16..25 are "011100100" → word 0x7200.
        assert_eq!(&stream[2..4], &[0x00, 0x72]);
    }

    #[test]
    fn encode_array_round_trip() {
        let code = example_code();
        let arr = code.encode_array();
        // 'a' = 97 is odd → high nibble of byte 48; 'e' = 101 → high nibble of byte 50.
        assert_eq!(arr[48] >> 4, 3);
        assert_eq!(arr[50] >> 4, 2);
        // 'l' = 108 is even → low nibble of byte 54.
        assert_eq!(arr[54] & 0x0F, 2);
        assert_eq!(HuffmanCode::from_encode_array(&arr).unwrap(), code);
    }

    #[test]
    fn rejects_invalid_codes() {
        let mut lengths = [0u8; 256];
        lengths[0] = 16;
        assert!(
            HuffmanCode::from_lengths(&lengths).is_err(),
            "longer than 15 bits"
        );
        lengths[0] = 1;
        lengths[1] = 1;
        lengths[2] = 1;
        assert!(HuffmanCode::from_lengths(&lengths).is_err(), "3 codes of 1 bit");
        let mut lengths = [0u8; 256];
        for l in lengths.iter_mut().take(5) {
            *l = 2;
        }
        assert!(HuffmanCode::from_lengths(&lengths).is_err(), "5 codes of 2 bits");
        assert!(HuffmanCode::from_encode_array(&[0u8; 127]).is_err());
    }

    /// A dominant symbol gets a 1-bit code, as in engine-written pages.
    #[test]
    fn one_bit_codes_decode() {
        let mut lengths = [0u8; 256];
        lengths[usize::from(b'0')] = 1;
        lengths[usize::from(b'x')] = 2;
        lengths[usize::from(b'1')] = 2;
        let code = HuffmanCode::from_lengths(&lengths).unwrap();
        let mut w = BitWriter::new();
        for b in *b"x0010" {
            let (c, l) = code.codeword(b).unwrap();
            w.push(c, l);
        }
        let bits = w.bit_len();
        assert_eq!(bits, 2 + 1 + 1 + 2 + 1);
        assert_eq!(code.decode(&w.into_bytes(), 0, bits).unwrap(), b"x0010");
    }

    #[test]
    fn hostile_streams_error() {
        let code = example_code();
        let stream = bits_to_stream(EXAMPLE_BITS);
        assert!(code.decode(&stream, 0, 1000).is_err(), "past the buffer");
        assert!(code.decode(&stream, 0, 14).is_err(), "truncated codeword");
        assert!(code.decode(&stream, 5, 4).is_err());
        // An incomplete code: bits "11..." never match when only 2-bit codes 00 and 01 exist.
        let mut lengths = [0u8; 256];
        lengths[usize::from(b'x')] = 2;
        lengths[usize::from(b'y')] = 2;
        let small = HuffmanCode::from_lengths(&lengths).unwrap();
        assert!(small.decode(&[0xFF, 0xFF], 0, 16).is_err());
        let empty = HuffmanCode::from_lengths(&[0u8; 256]).unwrap();
        assert_eq!(empty.decode(&[], 0, 0).unwrap(), b"");
        assert!(empty.decode(&[0, 0], 0, 2).is_err());
    }

    #[test]
    fn full_alphabet_round_trip() {
        // A complete code over all 256 symbols: every codeword 8 bits long.
        let lengths = [8u8; 256];
        let code = HuffmanCode::from_lengths(&lengths).unwrap();
        let text: Vec<u8> = (0..=255u8).rev().collect();
        let mut w = BitWriter::new();
        for &b in &text {
            let (c, l) = code.codeword(b).unwrap();
            w.push(c, l);
        }
        let bits = w.bit_len();
        assert_eq!(code.decode(&w.into_bytes(), 0, bits).unwrap(), text);
    }
}
