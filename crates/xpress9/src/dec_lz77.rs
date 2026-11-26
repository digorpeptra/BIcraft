// Ported from Microsoft XPress9 (MIT), dev/ese/src/_xpress9/Xpress9DecLz77.c@7030fe7407615160e54d152e4ef704eede2fdd7e
// and dev/ese/src/_xpress9/Xpress9Lz77Dec.i@7030fe7407615160e54d152e4ef704eede2fdd7e
// Copyright (c) Microsoft Corporation. Licensed under the MIT License.
// See LICENSE-MIT-microsoft-xpress9 in this crate and THIRD_PARTY_NOTICES.md at the repository root.

//! The XPress9 decoder: session state (`Xpress9DecoderStartSession`), block header parsing and
//! the decode loop of `Xpress9DecoderFetchDecompressedData`, and the LZ77 inner loop
//! (`Xpress9Lz77Dec.i`, here one function parameterised at run time instead of ten
//! preprocessor instantiations).
//!
//! Translation notes:
//! - Upstream is a push/pull streaming API (`Attach` compressed bytes, `Fetch` decompressed
//!   bytes) that stages input in an 8 KiB scratch area. Here each call receives a whole block,
//!   so the scratch staging is gone; the block format, the history buffer and its shifting, the
//!   MTF state, and every validity check are kept.
//! - The history buffer (`m_BufferData`, `2^max + 2^(max-1)` bytes) is a `Vec` whose length is
//!   the decode position; shifting it keeps the last `2^window` bytes exactly as upstream does.
//! - Hardening beyond upstream: a block whose window exceeds the decoder maximum is rejected
//!   (upstream would underflow when shifting), and after any error the session must be restarted.

use crate::Error;
use crate::dec_huffman::{
    DecodeEntry, huffman_create_decode_tables, huffman_decode_length_table, huffman_decode_symbol,
};
use crate::misc::{
    BitReader, HUFFMAN_MAX_CODEWORD_LENGTH, LZ77_LONG_LENGTH_ALPHABET_SIZE, LZ77_MAX_LONG_LENGTH,
    LZ77_MAX_SHORT_LENGTH, LZ77_MAX_SHORT_LENGTH_LOG, LZ77_SHORT_SYMBOL_ALPHABET_SIZE, XPRESS9_MAGIC,
    read_u32, xpress9_crc32,
};
use crate::{WINDOW_SIZE_LOG2_MAX, WINDOW_SIZE_LOG2_MIN};

pub(crate) const LZ77_SHORT_SYMBOL_DECODE_ROOT_BITS: usize = 12;
pub(crate) const LZ77_SHORT_SYMBOL_DECODE_TAIL_BITS: usize = 6;
const LZ77_SHORT_SYMBOL_DECODE_TAIL_TABLE_COUNT: usize = 64;
pub(crate) const LZ77_LONG_LENGTH_DECODE_ROOT_BITS: usize = 12;
pub(crate) const LZ77_LONG_LENGTH_DECODE_TAIL_BITS: usize = 6;
const LZ77_LONG_LENGTH_DECODE_TAIL_TABLE_COUNT: usize = 64;

pub(crate) const SHORT_TABLE_SIZE: usize = (1 << LZ77_SHORT_SYMBOL_DECODE_ROOT_BITS)
    + (1 << LZ77_SHORT_SYMBOL_DECODE_TAIL_BITS) * LZ77_SHORT_SYMBOL_DECODE_TAIL_TABLE_COUNT;
pub(crate) const LONG_TABLE_SIZE: usize = (1 << LZ77_LONG_LENGTH_DECODE_ROOT_BITS)
    + (1 << LZ77_LONG_LENGTH_DECODE_TAIL_BITS) * LZ77_LONG_LENGTH_DECODE_TAIL_TABLE_COUNT;

/// Size of an XPress9 block header in bytes (eight little-endian u32 words).
pub const BLOCK_HEADER_SIZE: usize = 32;

/// A parsed XPress9 block header (`LZ77_BLOCK_HEADER`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockHeader {
    /// Uncompressed size of the block in bytes.
    pub original_size: u32,
    /// Encoded size of the block **in bits**, including the 256-bit header itself.
    pub encoded_size_bits: u32,
    /// Raw flags word.
    pub flags: u32,
    /// Session signature (constant within a session).
    pub session_signature: u32,
    /// Block index within the session (0, 1, 2, ...).
    pub block_index: u32,
    /// Stored header checksum.
    pub header_crc: u32,
}

impl BlockHeader {
    /// Parse and validate the fixed parts of a header (magic, reserved word, CRC, reserved
    /// flag bits). Session consistency is checked by [`Decoder`].
    pub fn parse(input: &[u8]) -> Result<BlockHeader, Error> {
        let header = input
            .get(..BLOCK_HEADER_SIZE)
            .ok_or(Error::Truncated("block header"))?;
        let word = |i: usize| read_u32(header, i).ok_or(Error::Truncated("block header"));
        if word(0)? != XPRESS9_MAGIC {
            return Err(Error::CorruptedHeader("bad magic"));
        }
        if word(4)? != 0 {
            return Err(Error::CorruptedHeader("reserved word is not zero"));
        }
        let crc = xpress9_crc32(
            header.get(..28).ok_or(Error::Truncated("block header"))?,
            word(4)?,
        );
        if word(7)? != crc {
            return Err(Error::CorruptedHeader("header CRC mismatch"));
        }
        let h = BlockHeader {
            original_size: word(1)?,
            encoded_size_bits: word(2)?,
            flags: word(3)?,
            session_signature: word(5)?,
            block_index: word(6)?,
            header_crc: word(7)?,
        };
        if h.mtf_entry_count() > 4 {
            return Err(Error::CorruptedHeader("MTF entry count"));
        }
        if (h.flags >> 20) != 0 {
            return Err(Error::CorruptedHeader("reserved flag bits are not zero"));
        }
        if h.encoded_size_bits <= h.huffman_table_bits() + 8 * 4 * 8 {
            return Err(Error::CorruptedHeader("encoded size too small"));
        }
        Ok(h)
    }

    /// Bits 0-12: length of the encoded Huffman tables (and MTF state) in bits.
    pub fn huffman_table_bits(&self) -> u32 {
        self.flags & 0x1fff
    }
    /// Bits 13-15: log2 of the LZ77 window, minus 16.
    pub fn window_size_log2(&self) -> u32 {
        ((self.flags >> 13) & 7) + 16
    }
    /// Bits 16-17: number of MTF entries / 2.
    pub fn mtf_entry_count(&self) -> u32 {
        ((self.flags >> 16) & 3) << 1
    }
    /// Bit 18: minimum pointer match length (3 or 4).
    pub fn ptr_min_match_length(&self) -> u32 {
        ((self.flags >> 18) & 1) + 3
    }
    /// Bit 19: minimum MTF match length (2 or 3).
    pub fn mtf_min_match_length(&self) -> u32 {
        ((self.flags >> 19) & 1) + 2
    }
    /// Total size of the block in bytes, header included: `ceil(encoded_size_bits / 8)`.
    pub fn block_size(&self) -> usize {
        (self.encoded_size_bits as usize).div_ceil(8)
    }
}

/// `LZ77_DECODER_STATE_*`, reduced to what a whole-block API needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// `LZ77_DECODER_STATE_READY` / `WAIT_SESSION`: next block is block 0 of a session.
    WaitSession,
    /// `LZ77_DECODER_STATE_WAIT_BLOCK`.
    WaitBlock,
    /// A previous call failed; the session must be restarted.
    Failed,
}

/// `LZ77_MTF_STATE` (decoder side: positive distances).
#[derive(Debug, Clone, Copy, Default)]
struct MtfState {
    last_ptr: bool,
    offset: [usize; 4],
}

/// An XPress9 decoder (`XPRESS9_DECODER`). History is kept across blocks of a session, so
/// blocks must be decoded in order; [`Decoder::start_session`] forgets it.
pub struct Decoder {
    max_window_size_log2: u32,
    /// Nominal size of `m_BufferData`.
    buffer_size: usize,
    /// History buffer; `buf.len()` is the decode position.
    buf: Vec<u8>,
    state: State,

    window_size_log2: u32,
    mtf_entry_count: usize,
    ptr_min_match_length: usize,
    mtf_min_match_length: usize,
    session_signature: u32,
    block_index: u32,

    mtf: MtfState,
    tail_length: usize,
    tail_offset: usize,

    short_table: Vec<DecodeEntry>,
    long_table: Vec<DecodeEntry>,
    short_root: usize,
    long_root: usize,
}

impl Decoder {
    /// `Xpress9DecoderCreate` followed by `Xpress9DecoderStartSession`.
    ///
    /// `max_window_size_log2` must be in `16..=22`; use [`WINDOW_SIZE_LOG2_MAX`] to accept every
    /// stream the reference encoder can produce.
    pub fn new(max_window_size_log2: u32) -> Result<Decoder, Error> {
        if !(WINDOW_SIZE_LOG2_MIN..=WINDOW_SIZE_LOG2_MAX).contains(&max_window_size_log2) {
            return Err(Error::BadArgument("max window size log2 out of range"));
        }
        let buffer_size = (1usize << max_window_size_log2) + (1usize << (max_window_size_log2 - 1));
        Ok(Decoder {
            max_window_size_log2,
            buffer_size,
            buf: Vec::new(),
            state: State::WaitSession,
            window_size_log2: 0,
            mtf_entry_count: 0,
            ptr_min_match_length: 3,
            mtf_min_match_length: 2,
            session_signature: 0,
            block_index: 0,
            mtf: MtfState::default(),
            tail_length: 0,
            tail_offset: 0,
            short_table: vec![0; SHORT_TABLE_SIZE],
            long_table: vec![0; LONG_TABLE_SIZE],
            short_root: 0,
            long_root: 0,
        })
    }

    /// `Xpress9DecoderStartSession (fForceReset = TRUE)`: forget history and session state.
    pub fn start_session(&mut self) {
        self.buf.clear();
        self.state = State::WaitSession;
        self.mtf = MtfState::default();
        self.tail_length = 0;
        self.tail_offset = 0;
        self.block_index = 0;
        self.session_signature = 0;
    }

    /// Decode every block in `input` (which must end on a block boundary), appending at most
    /// `max_output` bytes to `out`. Returns the number of bytes appended.
    pub fn decode_blocks(
        &mut self,
        input: &[u8],
        out: &mut Vec<u8>,
        max_output: usize,
    ) -> Result<usize, Error> {
        let mut consumed = 0usize;
        let mut produced = 0usize;
        while consumed < input.len() {
            let rest = input.get(consumed..).ok_or(Error::Internal("input offset"))?;
            let before = out.len();
            let used = self.decode_block(rest, out, max_output.saturating_sub(produced))?;
            produced += out.len() - before;
            consumed += used;
        }
        Ok(produced)
    }

    /// Decode one block from the start of `input`, appending its bytes to `out`.
    ///
    /// Fails with [`Error::LimitExceeded`] if the block is larger than `max_output`. Returns
    /// the number of input bytes the block occupies.
    pub fn decode_block(
        &mut self,
        input: &[u8],
        out: &mut Vec<u8>,
        max_output: usize,
    ) -> Result<usize, Error> {
        if self.state == State::Failed {
            return Err(Error::BadArgument(
                "decoder session failed earlier; restart the session",
            ));
        }
        match self.decode_block_inner(input, out, max_output) {
            Ok(n) => Ok(n),
            Err(e) => {
                self.state = State::Failed;
                Err(e)
            }
        }
    }

    fn decode_block_inner(
        &mut self,
        input: &[u8],
        out: &mut Vec<u8>,
        max_output: usize,
    ) -> Result<usize, Error> {
        let header = BlockHeader::parse(input)?;

        if self.state == State::WaitSession {
            self.block_index = 0;
            self.session_signature = header.session_signature;
        }
        if header.block_index != self.block_index {
            return Err(Error::CorruptedHeader("unexpected block index"));
        }
        if header.session_signature != self.session_signature {
            return Err(Error::CorruptedHeader("session signature changed"));
        }

        if self.state == State::WaitSession {
            self.mtf_entry_count = header.mtf_entry_count() as usize;
            self.window_size_log2 = header.window_size_log2();
            self.ptr_min_match_length = header.ptr_min_match_length() as usize;
            self.mtf_min_match_length = header.mtf_min_match_length() as usize;
            if self.window_size_log2 > self.max_window_size_log2 {
                return Err(Error::WindowTooLarge(
                    self.window_size_log2,
                    self.max_window_size_log2,
                ));
            }
        } else if self.window_size_log2 != header.window_size_log2()
            || self.mtf_entry_count != header.mtf_entry_count() as usize
            || self.ptr_min_match_length != header.ptr_min_match_length() as usize
            || self.mtf_min_match_length != header.mtf_min_match_length() as usize
        {
            return Err(Error::CorruptedHeader("block parameters differ from the session"));
        }

        let orig_size = header.original_size as usize;
        if orig_size > max_output {
            return Err(Error::LimitExceeded(max_output));
        }
        let encoded_size_bits = u64::from(header.encoded_size_bits);
        let block_size = header.block_size();
        let block = input.get(..block_size).ok_or(Error::Truncated("block payload"))?;
        let payload = block
            .get(BLOCK_HEADER_SIZE..)
            .ok_or(Error::Truncated("block payload"))?;

        // LZ77_DECODER_STATE_WAIT_HUFFMAN
        let mut br = BitReader::new(payload);
        self.decode_state(&mut br)?;
        if br.consumed_bits() != u64::from(header.huffman_table_bits()) {
            return Err(Error::CorruptedData("Huffman table size mismatch"));
        }

        // LZ77_DECODER_STATE_DECODING
        let header_bits = 8 * BLOCK_HEADER_SIZE as u64;
        let mut block_decoded: usize = 0;
        loop {
            let decoded_bits = header_bits + br.consumed_bits();
            if block_decoded >= orig_size
                || block_decoded + self.tail_length >= orig_size
                || decoded_bits >= encoded_size_bits
            {
                // if we decoded everything, make sure we copied everything
                if block_decoded + self.tail_length != orig_size || decoded_bits != encoded_size_bits {
                    return Err(Error::CorruptedData("block size mismatch"));
                }
                if block_decoded == orig_size {
                    self.state = State::WaitBlock;
                    self.block_index = self.block_index.wrapping_add(1);
                    return Ok(block_size);
                }
                // otherwise a match tail is still pending: keep going
            }

            // Upstream decodes in slices sized by the compressed bytes at hand (at most 27 bits
            // per byte); keep the same pacing so a corrupt stream is caught early.
            let remaining_bytes = encoded_size_bits.saturating_sub(decoded_bits).div_ceil(8);
            let mut bytes_to_decode = usize::try_from(remaining_bytes * 8 / 27)
                .unwrap_or(usize::MAX)
                .max(1);
            bytes_to_decode = bytes_to_decode.min(orig_size.saturating_sub(block_decoded));

            // clamp by the space left in the buffer, shifting it if necessary
            let mut space = self.buffer_size.saturating_sub(self.buf.len());
            if space < 256 && space < bytes_to_decode {
                let keep = 1usize << self.window_size_log2;
                let drop = self
                    .buf
                    .len()
                    .checked_sub(keep)
                    .ok_or(Error::Internal("buffer shift"))?;
                self.buf.drain(..drop);
                space = self.buffer_size - self.buf.len();
            }
            bytes_to_decode = bytes_to_decode.min(space);

            let start = self.buf.len();
            let stop = start + bytes_to_decode;
            self.lz77_decode(&mut br, stop, self.buffer_size)?;
            let new_bytes = self.buf.get(start..).ok_or(Error::Internal("decode position"))?;
            block_decoded += new_bytes.len();
            if block_decoded > orig_size.saturating_add(self.buffer_size) {
                return Err(Error::Internal("runaway decode"));
            }
            out.extend_from_slice(new_bytes);
        }
    }

    /// `Xpress9DecodeState`: MTF state and both Huffman tables.
    fn decode_state(&mut self, br: &mut BitReader<'_>) -> Result<(), Error> {
        if self.mtf_entry_count > 0 {
            self.mtf.last_ptr = br.load(1) != 0;
            for i in 0..self.mtf_entry_count {
                let msb = br.load(5);
                if msb >= self.window_size_log2 {
                    return Err(Error::CorruptedData("MTF offset larger than the window"));
                }
                let offset = br.load(msb) as usize + (1usize << msb);
                *self.mtf.offset.get_mut(i).ok_or(Error::Internal("MTF index"))? = offset;
            }
        }

        let mut short_symbol_length = [0u8; LZ77_SHORT_SYMBOL_ALPHABET_SIZE];
        huffman_decode_length_table(
            br,
            &mut short_symbol_length,
            LZ77_MAX_SHORT_LENGTH,
            HUFFMAN_MAX_CODEWORD_LENGTH,
        )?;
        self.short_root = huffman_create_decode_tables(
            &short_symbol_length,
            &mut self.short_table,
            LZ77_SHORT_SYMBOL_DECODE_ROOT_BITS,
            LZ77_SHORT_SYMBOL_DECODE_TAIL_BITS,
        )?;

        let mut long_length_length = [0u8; LZ77_LONG_LENGTH_ALPHABET_SIZE];
        huffman_decode_length_table(
            br,
            &mut long_length_length,
            LZ77_MAX_SHORT_LENGTH,
            HUFFMAN_MAX_CODEWORD_LENGTH,
        )?;
        self.long_root = huffman_create_decode_tables(
            &long_length_length,
            &mut self.long_table,
            LZ77_LONG_LENGTH_DECODE_ROOT_BITS,
            LZ77_LONG_LENGTH_DECODE_TAIL_BITS,
        )?;
        Ok(())
    }

    /// `Xpress9Lz77Dec`: decode until the decode position reaches `stop` (a match may run past
    /// it), never writing at or beyond `end_of_buffer`; a match cut by the buffer end is kept
    /// as the "tail".
    fn lz77_decode(
        &mut self,
        br: &mut BitReader<'_>,
        stop: usize,
        end_of_buffer: usize,
    ) -> Result<(), Error> {
        let mtf_count = self.mtf_entry_count;
        let ptr_min = self.ptr_min_match_length;
        let mtf_min = self.mtf_min_match_length;
        let Decoder {
            buf,
            short_table,
            long_table,
            short_root,
            long_root,
            mtf,
            tail_length,
            tail_offset,
            ..
        } = self;
        let (short_root, long_root) = (*short_root, *long_root);

        if !(buf.len() < stop && stop <= end_of_buffer) {
            return Err(Error::Internal("bad decode window"));
        }

        let mut last_ptr = mtf.last_ptr;
        let [mut off0, mut off1, mut off2, mut off3] = mtf.offset;

        let mut tail: Option<(usize, usize)> = None;

        // finish a match cut short by the end of the buffer
        if *tail_length != 0 {
            let length = *tail_length;
            let offset = *tail_offset;
            if offset > buf.len() {
                return Err(Error::CorruptedData("match offset before start of data"));
            }
            let last = buf.len() + length;
            if last > end_of_buffer {
                let n = end_of_buffer - buf.len();
                copy_match(buf, offset, n)?;
                tail = Some((last - end_of_buffer, offset));
            } else {
                copy_match(buf, offset, length)?;
            }
            if tail.is_some() || buf.len() >= stop {
                *tail_length = tail.map_or(0, |t| t.0);
                *tail_offset = tail.map_or(0, |t| t.1);
                return Ok(());
            }
        }

        'outer: loop {
            // literals
            let mut symbol;
            loop {
                symbol = huffman_decode_symbol(
                    br,
                    short_table,
                    short_root,
                    LZ77_SHORT_SYMBOL_DECODE_ROOT_BITS as u32,
                    LZ77_SHORT_SYMBOL_DECODE_TAIL_BITS as u32,
                )?;
                if symbol >= 256 {
                    break;
                }
                buf.push(symbol as u8);
                last_ptr = false;
                if buf.len() >= stop {
                    break 'outer;
                }
            }
            symbol -= 256;

            // a pointer: decode the length first
            let mut length = symbol & (LZ77_MAX_SHORT_LENGTH - 1);
            symbol >>= LZ77_MAX_SHORT_LENGTH_LOG;
            if length == LZ77_MAX_SHORT_LENGTH - 1 {
                length = huffman_decode_symbol(
                    br,
                    long_table,
                    long_root,
                    LZ77_LONG_LENGTH_DECODE_ROOT_BITS as u32,
                    LZ77_LONG_LENGTH_DECODE_TAIL_BITS as u32,
                )?;
                if length >= LZ77_MAX_LONG_LENGTH {
                    let bits = (length - LZ77_MAX_LONG_LENGTH) as u32;
                    length = br.load(bits) as usize;
                    length += 1usize << bits;
                    length += LZ77_MAX_LONG_LENGTH - 1;
                }
                length += LZ77_MAX_SHORT_LENGTH - 1;
            }

            let offset: usize;
            if mtf_count > 0 && symbol < mtf_count {
                // an MTF pointer
                length += mtf_min;
                if last_ptr {
                    if mtf_count <= 2 {
                        if symbol != 0 {
                            return Err(Error::CorruptedData("bad MTF index after a pointer"));
                        }
                        offset = off1;
                    } else if symbol == 0 {
                        offset = off1;
                    } else if symbol == 1 {
                        offset = off2;
                        off2 = off1;
                    } else if symbol == 2 {
                        offset = off3;
                        off3 = off2;
                        off2 = off1;
                    } else {
                        return Err(Error::CorruptedData("bad MTF index after a pointer"));
                    }
                    off1 = off0;
                    off0 = offset;
                } else if symbol == 0 {
                    offset = off0;
                } else if mtf_count <= 2 || symbol == 1 {
                    offset = off1;
                    off1 = off0;
                    off0 = offset;
                } else if symbol == 2 {
                    offset = off2;
                    off2 = off1;
                    off1 = off0;
                    off0 = offset;
                } else {
                    offset = off3;
                    off3 = off2;
                    off2 = off1;
                    off1 = off0;
                    off0 = offset;
                }
            } else {
                // a regular pointer: decode the offset
                length += ptr_min;
                let bits = (symbol - mtf_count) as u32;
                if bits > 31 {
                    return Err(Error::CorruptedData("offset width"));
                }
                offset = br.load(bits) as usize + (1usize << bits);
                if mtf_count > 2 {
                    off3 = off2;
                    off2 = off1;
                }
                if mtf_count > 0 {
                    off1 = off0;
                    off0 = offset;
                }
            }

            // got length and offset, copy the data
            if offset > buf.len() {
                return Err(Error::CorruptedData("match offset before start of data"));
            }
            last_ptr = true;

            let last = buf.len() + length;
            if last > end_of_buffer {
                let n = end_of_buffer - buf.len();
                copy_match(buf, offset, n)?;
                tail = Some((last - end_of_buffer, offset));
                break 'outer;
            }
            copy_match(buf, offset, length)?;

            if buf.len() >= stop {
                break;
            }
        }

        mtf.last_ptr = last_ptr;
        mtf.offset = [off0, off1, off2, off3];
        *tail_length = tail.map_or(0, |t| t.0);
        *tail_offset = tail.map_or(0, |t| t.1);
        Ok(())
    }
}

/// Byte-wise LZ77 copy (`pDst[i] = pDst[i - offset]`), overlap allowed.
#[inline]
fn copy_match(buf: &mut Vec<u8>, offset: usize, length: usize) -> Result<(), Error> {
    let start = buf
        .len()
        .checked_sub(offset)
        .ok_or(Error::CorruptedData("match offset before start of data"))?;
    if offset == 0 {
        return Err(Error::CorruptedData("zero match offset"));
    }
    let mut remaining = length;
    while remaining > 0 {
        // the source period is `offset`; each round can copy everything written since `start`
        let n = remaining.min(buf.len() - start);
        buf.extend_from_within(start..start + n);
        remaining -= n;
    }
    Ok(())
}
