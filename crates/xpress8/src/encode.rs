//! The encoder (BIcraft's own, written from the token format of [MS-XCA] §2.3/§2.4): a greedy
//! Plain LZ77 compressor and the chunk framing. BIcraft never writes Power Pivot workbooks; the
//! encoder builds synthetic test backups and checks the decoder on round trips.

use crate::decode::MAX_CHUNK_SIZE;

/// Largest match offset of the format (13 bits, plus one).
const MAX_OFFSET: usize = 1 << 13;
/// Shortest match.
const MIN_MATCH: usize = 3;
/// Positions remembered per hash bucket: one (the most recent).
const HASH_BITS: u32 = 13;

/// Uncompressed bytes per chunk written by [`compress_chunked`].
pub const DEFAULT_CHUNK_SIZE: usize = 1 << 15;

fn hash3(data: &[u8], at: usize) -> Option<usize> {
    let b = data.get(at..at.checked_add(MIN_MATCH)?)?;
    let (&a, &b1, &c) = (b.first()?, b.get(1)?, b.get(2)?);
    let v = u32::from(a) | (u32::from(b1) << 8) | (u32::from(c) << 16);
    usize::try_from(v.wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)).ok()
}

struct Writer {
    out: Vec<u8>,
    flags: u32,
    flag_count: u32,
    flag_at: usize,
    /// Position of the byte whose high nibble the next long match fills.
    half_byte: Option<usize>,
}

impl Writer {
    fn new(capacity: usize) -> Self {
        Writer {
            out: Vec::with_capacity(capacity),
            flags: 0,
            flag_count: 0,
            flag_at: 0,
            half_byte: None,
        }
    }

    /// Starts a new flag word when the current one has its 32 flags.
    fn flag(&mut self, bit: bool) {
        if self.flag_count == 32 {
            self.flush_flags();
            self.flag_at = self.out.len();
            self.out.extend_from_slice(&[0; 4]);
            self.flags = 0;
            self.flag_count = 0;
        }
        self.flags = (self.flags << 1) | u32::from(bit);
        self.flag_count += 1;
    }

    /// Writes the current flag word; unused low bits are set to 1, as [MS-XCA] §2.3 does.
    fn flush_flags(&mut self) {
        let unused = 32 - self.flag_count;
        let word = if unused == 32 {
            u32::MAX
        } else {
            (self.flags << unused) | ((1u32 << unused) - 1)
        };
        if let Some(slot) = self.out.get_mut(self.flag_at..self.flag_at + 4) {
            slot.copy_from_slice(&word.to_le_bytes());
        }
    }

    fn literal(&mut self, byte: u8) {
        self.flag(false);
        self.out.push(byte);
    }

    /// A match of `length` ≥ 3 bytes, `offset` (1..=8192) back.
    fn matched(&mut self, offset: usize, length: usize) {
        self.flag(true);
        let extra = length - MIN_MATCH;
        let code = u16::try_from(((offset - 1) << 3) | extra.min(7)).unwrap_or(0);
        self.out.extend_from_slice(&code.to_le_bytes());
        if extra < 7 {
            return;
        }
        let rest = extra - 7;
        let nibble = u8::try_from(rest.min(15)).unwrap_or(15);
        match self.half_byte.take() {
            None => {
                self.half_byte = Some(self.out.len());
                self.out.push(nibble);
            }
            Some(at) => {
                if let Some(b) = self.out.get_mut(at) {
                    *b |= nibble << 4;
                }
            }
        }
        if rest < 15 {
            return;
        }
        let rest = rest - 15;
        if rest < 255 {
            self.out.push(u8::try_from(rest).unwrap_or(0));
            return;
        }
        self.out.push(255);
        // The full length minus 3: a u16, or a zero u16 then a u32.
        match u16::try_from(extra) {
            Ok(full) => self.out.extend_from_slice(&full.to_le_bytes()),
            Err(_) => {
                self.out.extend_from_slice(&0u16.to_le_bytes());
                let full = u32::try_from(extra).unwrap_or(u32::MAX);
                self.out.extend_from_slice(&full.to_le_bytes());
            }
        }
    }
}

/// One Plain LZ77 block holding `data` (greedy matching, one candidate per hash). The block
/// decodes back to `data` when the decoder is told its size, `data.len()`.
pub fn compress_block(data: &[u8]) -> Vec<u8> {
    let mut w = Writer::new(data.len() + data.len() / 8 + 8);
    w.out.extend_from_slice(&[0; 4]);
    let mut head = vec![usize::MAX; 1 << HASH_BITS];
    let mut i = 0usize;
    while i < data.len() {
        let mut best = (0usize, 0usize);
        if let Some(h) = hash3(data, i)
            && let Some(slot) = head.get_mut(h)
        {
            let candidate = *slot;
            *slot = i;
            if candidate != usize::MAX && i - candidate <= MAX_OFFSET {
                let length = data
                    .get(i..)
                    .unwrap_or_default()
                    .iter()
                    .zip(data.get(candidate..).unwrap_or_default())
                    .take_while(|(a, b)| a == b)
                    .count();
                if length >= MIN_MATCH {
                    best = (i - candidate, length);
                }
            }
        }
        match best {
            (offset, length) if length >= MIN_MATCH => {
                w.matched(offset, length);
                for p in i + 1..i + length {
                    if let Some(h) = hash3(data, p)
                        && let Some(slot) = head.get_mut(h)
                    {
                        *slot = p;
                    }
                }
                i += length;
            }
            _ => {
                w.literal(data.get(i).copied().unwrap_or_default());
                i += 1;
            }
        }
    }
    w.flush_flags();
    w.out
}

/// `data` as a chunked file (see [`crate::decompress_chunked`]): chunks of at most
/// `chunk_size` bytes (clamped to 1..=65535), each compressed, or stored verbatim when
/// compression does not make it smaller.
pub fn compress_chunked(data: &[u8], chunk_size: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 2 + 16);
    for part in data.chunks(chunk_size.clamp(1, MAX_CHUNK_SIZE)) {
        let size = u16::try_from(part.len()).unwrap_or(u16::MAX);
        let packed = compress_block(part);
        out.extend_from_slice(&size.to_le_bytes());
        match u16::try_from(packed.len()) {
            Ok(n) if packed.len() < part.len() => {
                out.extend_from_slice(&n.to_le_bytes());
                out.extend_from_slice(&packed);
            }
            _ => {
                out.extend_from_slice(&size.to_le_bytes());
                out.extend_from_slice(part);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decompress_block, decompress_chunked};

    fn round_trip(data: &[u8]) {
        let block = compress_block(data);
        assert_eq!(decompress_block(&block, data.len()).unwrap(), data);
        let chunked = compress_chunked(data, DEFAULT_CHUNK_SIZE);
        assert_eq!(
            decompress_chunked(&chunked, Some(data.len() as u64), 1 << 30).unwrap(),
            data
        );
    }

    #[test]
    fn blocks_and_chunks_round_trip() {
        round_trip(b"");
        round_trip(b"a");
        round_trip(b"abcabcabcabcabcabcabc");
        round_trip(&vec![7u8; 100_000]);
        let text: Vec<u8> = (0..50_000u32)
            .flat_map(|i| format!("row {} value {};", i % 97, i % 13).into_bytes())
            .collect();
        round_trip(&text);
        let mut state = 0x1234_5678_9ABC_DEF0u64;
        let noise: Vec<u8> = (0..70_000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 32) as u8
            })
            .collect();
        round_trip(&noise);
    }

    #[test]
    fn every_length_form_round_trips() {
        // Matches of every length class, two long ones in a row sharing a nibble byte.
        for n in [3usize, 9, 10, 24, 25, 60, 279, 280, 300, 4000, 65_000] {
            let mut data = b"xy".to_vec();
            data.extend(std::iter::repeat_n(b'z', n));
            data.extend_from_slice(b"xy");
            data.extend(std::iter::repeat_n(b'z', n + 1));
            let block = compress_block(&data);
            assert_eq!(decompress_block(&block, data.len()).unwrap(), data, "{n}");
        }
    }

    #[test]
    fn incompressible_chunks_are_stored() {
        let data = b"abcdefgh";
        let chunked = compress_chunked(data, 4);
        assert_eq!(&chunked[..4], &[4, 0, 4, 0]);
        assert_eq!(decompress_chunked(&chunked, Some(8), 64).unwrap(), data);
    }
}
