// Ported from DashCraft (MIT OR Apache-2.0), crates/pbi-vertipaq/src/xpress8.rs@0958fd82b04656ee1797101a2b4df8d114dfebd1
// Copyright (c) 2026 Jack Sweeney and the DashCraft contributors. Used under the MIT License.
// See LICENSE-MIT-dashcraft in this crate and THIRD_PARTY_NOTICES.md at the repository root.

//! The decoder: one Plain LZ77 block ([MS-XCA] §2.4) and a chunked file of an Excel Power Pivot
//! backup (`docs/formats/powerpivot.md`).
//!
//! Translation notes:
//! - Upstream's error type is kept with its variants; the chunk framing is unchanged.
//! - The block decoder was checked line by line against the decompression pseudocode of
//!   [MS-XCA] §2.4.1 (flag words read MSB first, the shared length nibble, the 255 → `u16` →
//!   `u32` length escapes, the `≥ 22` check of the long form). Unlike the pseudocode, which ends
//!   at the end of the input, it stops once the declared output size is reached (the size every
//!   chunk header carries), and a final match is truncated to that size.
//! - Every input is untrusted: sizes are checked before allocating, offsets must stay inside the
//!   current chunk's output, output never exceeds the declared sizes.

/// Size of a chunk header (two little-endian `u16`).
pub const CHUNK_HEADER_LEN: usize = 4;

/// Largest output one chunk can declare.
pub const MAX_CHUNK_SIZE: usize = u16::MAX as usize;

/// Errors produced while decompressing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The input ended inside a chunk header or payload.
    #[error("chunk {chunk} at offset {offset}: input truncated")]
    Truncated {
        /// Index of the chunk.
        chunk: usize,
        /// Byte offset of the chunk header in the input.
        offset: usize,
    },
    /// A chunk declares a payload larger than its output.
    #[error(
        "chunk {chunk} at offset {offset}: compressed size {compressed} exceeds uncompressed size {uncompressed}"
    )]
    InvalidChunk {
        chunk: usize,
        offset: usize,
        compressed: usize,
        uncompressed: usize,
    },
    /// The LZ77 stream of a chunk is malformed.
    #[error("chunk {chunk}: {message}")]
    Corrupt {
        /// Index of the chunk.
        chunk: usize,
        /// What is wrong (no content).
        message: String,
    },
    /// The decompressed size differs from the size recorded in the backup log.
    #[error("decompressed {actual} bytes, the backup log records {expected}")]
    SizeMismatch {
        /// Size from the backup log.
        expected: u64,
        /// Bytes produced (or about to be produced).
        actual: u64,
    },
    /// The output would exceed the caller's cap.
    #[error("decompressed size exceeds the limit of {limit} bytes")]
    LimitExceeded {
        /// The cap.
        limit: u64,
    },
}

/// Error of a block decode (without chunk context).
#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockError(String);

fn corrupt(message: impl Into<String>) -> BlockError {
    BlockError(message.into())
}

struct Input<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Input<'_> {
    fn u8(&mut self) -> Result<u8, BlockError> {
        let b = *self
            .bytes
            .get(self.pos)
            .ok_or_else(|| corrupt("input ends inside a token"))?;
        self.pos += 1;
        Ok(b)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], BlockError> {
        let end = self.pos.checked_add(N).filter(|&e| e <= self.bytes.len());
        let slice = end
            .and_then(|end| self.bytes.get(self.pos..end))
            .ok_or_else(|| corrupt("input ends inside a token"))?;
        let mut out = [0u8; N];
        out.copy_from_slice(slice);
        self.pos += N;
        Ok(out)
    }
}

/// Decodes one Plain LZ77 block, appending exactly `size` bytes to `out`. Matches may only
/// reference bytes produced by this block. A final match that runs past `size` is truncated.
fn decode_block_into(input: &[u8], size: usize, out: &mut Vec<u8>) -> Result<(), BlockError> {
    let start = out.len();
    let end = start.checked_add(size).ok_or_else(|| corrupt("size overflow"))?;
    out.try_reserve(size)
        .map_err(|_| corrupt("cannot allocate the chunk output"))?;
    let mut input = Input { bytes: input, pos: 0 };
    let mut flags: u32 = 0;
    let mut flag_count: u32 = 0;
    let mut half_byte: Option<usize> = None;
    while out.len() < end {
        if flag_count == 0 {
            flags = u32::from_le_bytes(input.array::<4>()?);
            flag_count = 32;
        }
        flag_count -= 1;
        if flags & (1u32 << flag_count) == 0 {
            out.push(input.u8()?);
            continue;
        }
        let token = u16::from_le_bytes(input.array::<2>()?);
        let offset = usize::from(token >> 3) + 1;
        let mut length = usize::from(token & 7);
        if length == 7 {
            length = match half_byte.take() {
                None => {
                    half_byte = Some(input.pos);
                    usize::from(input.u8()? & 0x0F)
                }
                Some(at) => usize::from(input.bytes.get(at).copied().unwrap_or_default() >> 4),
            };
            if length == 15 {
                length = usize::from(input.u8()?);
                if length == 255 {
                    let mut full = u64::from(u16::from_le_bytes(input.array::<2>()?));
                    if full == 0 {
                        full = u64::from(u32::from_le_bytes(input.array::<4>()?));
                    }
                    if full < 15 + 7 {
                        return Err(corrupt(format!("invalid long match length {full}")));
                    }
                    length =
                        usize::try_from(full - (15 + 7)).map_err(|_| corrupt("match length overflow"))?;
                }
                length = length
                    .checked_add(15)
                    .ok_or_else(|| corrupt("match length overflow"))?;
            }
            length += 7;
        }
        length = length
            .checked_add(3)
            .ok_or_else(|| corrupt("match length overflow"))?;
        let produced = out.len() - start;
        if offset > produced {
            return Err(corrupt(format!(
                "match offset {offset} reaches before the start of the block ({produced} bytes decoded)"
            )));
        }
        let length = length.min(end - out.len());
        let from = out.len() - offset;
        if offset >= length {
            out.extend_from_within(from..from + length);
        } else {
            for i in 0..length {
                let byte = out.get(from + i).copied().unwrap_or_default();
                out.push(byte);
            }
        }
    }
    Ok(())
}

/// Decodes one standalone Plain LZ77 block of `size` output bytes.
///
/// # Errors
/// [`Error::Corrupt`] (chunk 0) for malformed input.
pub fn decompress_block(input: &[u8], size: usize) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    decode_block_into(input, size, &mut out)
        .map_err(|BlockError(message)| Error::Corrupt { chunk: 0, message })?;
    Ok(out)
}

/// Decompresses a chunked file: chunks `u16 LE uncompressed size, u16 LE compressed size,
/// payload`, the payload stored verbatim when both sizes are equal, else one independent Plain
/// LZ77 block.
///
/// `expected` is the size from the backup log, if known; `max_output` caps the output
/// regardless. Memory grows only with chunks actually decoded.
///
/// # Errors
/// Any [`Error`].
pub fn decompress_chunked(input: &[u8], expected: Option<u64>, max_output: u64) -> Result<Vec<u8>, Error> {
    if let Some(expected) = expected {
        if expected > max_output {
            return Err(Error::LimitExceeded { limit: max_output });
        }
        // Each chunk produces at most 65535 bytes from at least 4 input bytes.
        let possible = (input.len() as u64 / CHUNK_HEADER_LEN as u64) * MAX_CHUNK_SIZE as u64;
        if expected > possible {
            return Err(Error::SizeMismatch {
                expected,
                actual: possible,
            });
        }
    }
    let limit = expected.unwrap_or(max_output).min(max_output);
    let mut out = Vec::new();
    if let Some(expected) = expected {
        let hint = usize::try_from(expected.min(64 << 20)).unwrap_or(0);
        out.try_reserve(hint)
            .map_err(|_| Error::LimitExceeded { limit: expected })?;
    }
    let mut pos = 0usize;
    let mut chunk = 0usize;
    while pos < input.len() {
        let header = input
            .get(pos..pos + CHUNK_HEADER_LEN)
            .ok_or(Error::Truncated { chunk, offset: pos })?;
        let (Some(&u0), Some(&u1), Some(&c0), Some(&c1)) =
            (header.first(), header.get(1), header.get(2), header.get(3))
        else {
            return Err(Error::Truncated { chunk, offset: pos });
        };
        let uncompressed = usize::from(u16::from_le_bytes([u0, u1]));
        let compressed = usize::from(u16::from_le_bytes([c0, c1]));
        let payload_start = pos + CHUNK_HEADER_LEN;
        let payload = input
            .get(payload_start..payload_start + compressed)
            .ok_or(Error::Truncated { chunk, offset: pos })?;
        if compressed > uncompressed {
            return Err(Error::InvalidChunk {
                chunk,
                offset: pos,
                compressed,
                uncompressed,
            });
        }
        let total = out.len() as u64 + uncompressed as u64;
        if total > limit {
            return Err(match expected {
                Some(expected) if expected <= max_output => Error::SizeMismatch {
                    expected,
                    actual: total,
                },
                _ => Error::LimitExceeded { limit: max_output },
            });
        }
        if compressed == uncompressed {
            out.extend_from_slice(payload);
        } else {
            decode_block_into(payload, uncompressed, &mut out)
                .map_err(|BlockError(message)| Error::Corrupt { chunk, message })?;
        }
        pos = payload_start + compressed;
        chunk += 1;
    }
    if let Some(expected) = expected
        && out.len() as u64 != expected
    {
        return Err(Error::SizeMismatch {
            expected,
            actual: out.len() as u64,
        });
    }
    Ok(out)
}

/// Fuzzing entry point: decodes `data` as a chunked file and as a single block. Never panics.
pub fn fuzz(data: &[u8]) {
    let _ = decompress_chunked(data, None, 1 << 20);
    if let Some((&size, rest)) = data.split_first() {
        let _ = decompress_block(rest, usize::from(size) * 64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frames `content` as stored chunks (no compression) of at most `chunk` bytes.
    fn stored_chunks(content: &[u8], chunk: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for part in content.chunks(chunk.clamp(1, MAX_CHUNK_SIZE)) {
            let len = u16::try_from(part.len()).unwrap();
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(part);
        }
        out
    }

    /// A payload: `literal` then one match copying `copy` bytes from `offset` back.
    fn literal_then_match(literal: &[u8], offset: usize, copy: usize) -> Vec<u8> {
        assert!(literal.len() < 31 && copy >= 3 && (1..=8192).contains(&offset));
        let flags: u32 = 1u32 << (31 - literal.len());
        let mut out = flags.to_le_bytes().to_vec();
        out.extend_from_slice(literal);
        let extra = copy - 3;
        let token = u16::try_from(((offset - 1) << 3) | extra.min(7)).unwrap();
        out.extend_from_slice(&token.to_le_bytes());
        if extra >= 7 {
            let rest = extra - 7;
            if rest < 15 {
                out.push(u8::try_from(rest).unwrap());
            } else {
                out.push(15);
                let rest = rest - 15;
                if rest < 255 {
                    out.push(u8::try_from(rest).unwrap());
                } else {
                    out.push(255);
                    out.extend_from_slice(&u16::try_from(extra).unwrap().to_le_bytes());
                }
            }
        }
        out
    }

    fn chunk(uncompressed: usize, payload: &[u8]) -> Vec<u8> {
        let mut out = u16::try_from(uncompressed).unwrap().to_le_bytes().to_vec();
        out.extend_from_slice(&u16::try_from(payload.len()).unwrap().to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn decodes_literals_and_overlapping_matches() {
        let payload = literal_then_match(b"abc", 3, 9);
        assert_eq!(decompress_block(&payload, 12).unwrap(), b"abcabcabcabc");
        let payload = literal_then_match(b"x", 1, 5);
        assert_eq!(decompress_block(&payload, 6).unwrap(), b"xxxxxx");
    }

    #[test]
    fn decodes_every_length_extension() {
        for copy in [3usize, 9, 10, 24, 25, 60, 300, 4000] {
            let payload = literal_then_match(b"ab", 2, copy);
            let out = decompress_block(&payload, 2 + copy).unwrap();
            assert_eq!(out.len(), 2 + copy, "copy {copy}");
            assert!(out.chunks(2).all(|p| p[0] == b'a'), "copy {copy}");
        }
    }

    #[test]
    fn decodes_the_u32_length_form() {
        // [MS-XCA] §2.4.1: a zero u16 after the 255 byte is followed by the u32 length.
        let copy = 70_000usize;
        let flags: u32 = 1u32 << 30;
        let mut payload = flags.to_le_bytes().to_vec();
        payload.push(b'q');
        payload.extend_from_slice(&7u16.to_le_bytes());
        payload.extend_from_slice(&[15, 255, 0, 0]);
        payload.extend_from_slice(&u32::try_from(copy - 3).unwrap().to_le_bytes());
        let out = decompress_block(&payload, 1 + copy).unwrap();
        assert_eq!(out, vec![b'q'; 1 + copy]);
    }

    #[test]
    fn shares_the_length_nibble_between_two_long_matches() {
        let flags: u32 = 0b011 << 29;
        let mut payload = flags.to_le_bytes().to_vec();
        payload.push(b'z');
        payload.extend_from_slice(&7u16.to_le_bytes());
        payload.push(0x21); // low nibble 1 → 3+7+1 = 11; high nibble 2 → 12
        payload.extend_from_slice(&7u16.to_le_bytes());
        let out = decompress_block(&payload, 1 + 11 + 12).unwrap();
        assert_eq!(out, vec![b'z'; 24]);
    }

    #[test]
    fn chunked_files_mix_stored_and_compressed_chunks() {
        let mut data = stored_chunks(b"hello ", 4);
        data.extend(chunk(12, &literal_then_match(b"abc", 3, 9)));
        data.extend(chunk(0, &[]));
        let out = decompress_chunked(&data, Some(18), 1 << 20).unwrap();
        assert_eq!(out, b"hello abcabcabcabc");
        assert_eq!(decompress_chunked(&[], Some(0), 10).unwrap(), b"");
    }

    #[test]
    fn rejects_malformed_and_lying_input() {
        let good = chunk(12, &literal_then_match(b"abc", 3, 9));
        assert!(matches!(
            decompress_chunked(&good, Some(13), 1 << 20),
            Err(Error::SizeMismatch { .. })
        ));
        assert!(matches!(
            decompress_chunked(&good[..good.len() - 1], None, 1 << 20),
            Err(Error::Truncated { .. })
        ));
        assert!(matches!(
            decompress_chunked(&good[..3], None, 1 << 20),
            Err(Error::Truncated { .. })
        ));
        assert!(matches!(
            decompress_chunked(&chunk(2, b"abc"), None, 1 << 20),
            Err(Error::InvalidChunk { .. })
        ));
        assert!(matches!(
            decompress_chunked(&chunk(10, &literal_then_match(b"a", 5, 9)), None, 1 << 20),
            Err(Error::Corrupt { .. })
        ));
        assert!(matches!(
            decompress_chunked(&good, None, 5),
            Err(Error::LimitExceeded { .. })
        ));
        assert!(matches!(
            decompress_chunked(&good, Some(1 << 40), 1 << 50),
            Err(Error::SizeMismatch { .. })
        ));
        assert!(decompress_block(&literal_then_match(b"abc", 3, 3), 100).is_err());
        // The long form must hold at least 22.
        let mut bad = (1u32 << 30).to_le_bytes().to_vec();
        bad.push(b'a');
        bad.extend_from_slice(&7u16.to_le_bytes());
        bad.extend_from_slice(&[15, 255, 21, 0]);
        assert!(decompress_block(&bad, 100).is_err());
    }
}
