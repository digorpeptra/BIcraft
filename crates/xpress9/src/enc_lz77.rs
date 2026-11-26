// Ported from Microsoft XPress9 (MIT), dev/ese/src/_xpress9/Xpress9EncLz77.c@7030fe7407615160e54d152e4ef704eede2fdd7e
// together with Xpress9Lz77EncPass1.i, Xpress9Lz77EncPass2.i, Xpress9Lz77EncInsert.i, Xpress9Lookup.i,
// xpress9lookup.i2 and xpress9lookup.i3 (same directory and commit).
// Copyright (c) Microsoft Corporation. Licensed under the MIT License.
// See LICENSE-MIT-microsoft-xpress9 in this crate and THIRD_PARTY_NOTICES.md at the repository root.

//! The XPress9 encoder.
//!
//! Translation notes:
//! - Upstream instantiates pass 1 thirty times through the preprocessor (MTF count, minimum
//!   match lengths, deep lookup, lazy evaluation); here it is one function driven by the
//!   session parameters, with the same control flow.
//! - The upstream Attach / Compress / FetchCompressedData / Detach sequence is folded into
//!   [`Encoder::compress`], which returns the finished blocks. The scratch-area staging used
//!   to hand bytes to the caller in pieces is dropped; the bit stream is identical.
//! - `Xpress9Lz77EncInsert_SSE2` (selected by [`FLAG_USE_SSE2`] with a minimum pointer length
//!   of 4) is reproduced with scalar code: it hashes 32-bit lanes and differs from the plain
//!   insertion, so it changes which matches are found (not the format). The data buffer is
//!   assumed 16-byte aligned, as upstream's 256-byte aligned allocation is. Statistics counters
//!   that do not affect the output are not kept.
//! - The session signature comes from `__rdtsc()` upstream; here the caller passes the 64-bit
//!   value (see [`Encoder::start_session`]), which makes the output reproducible.

use crate::Error;
use crate::enc_huffman::{HuffmanScratch, xpress9_huffman_create_and_encode_table};
use crate::misc::{
    BitWriter, HUFFMAN_MAX_CODEWORD_LENGTH, LZ77_LONG_LENGTH_ALPHABET_SIZE, LZ77_MAX_LONG_LENGTH,
    LZ77_MAX_MTF, LZ77_MAX_SHORT_LENGTH, LZ77_MAX_SHORT_LENGTH_LOG, LZ77_SHORT_SYMBOL_ALPHABET_SIZE,
    XPRESS9_MAGIC, get_msb, xpress9_crc32,
};
use crate::zobrist::ZOBRIST_TABLE;
use crate::{WINDOW_SIZE_LOG2_MAX, WINDOW_SIZE_LOG2_MIN};

/// `Xpress9Flag_UseSSE2`: use the hash insertion of the SSE2 build (only matters with a
/// minimum pointer match length of 4).
pub const FLAG_USE_SSE2: u32 = 1;

/// `XPRESS9_ENCODER_PARAMS` (without `m_cbSize`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderParams {
    /// Upper bound of the session's total input; 0 means "no limit". A small value shrinks
    /// the window and the hash table.
    pub max_stream_length: u32,
    /// Number of MTF entries: 0, 2 or 4.
    pub mtf_entry_count: u32,
    /// Match-finder depth, 0..=9 (unbounded when `optimization_level` is 1).
    pub lookup_depth: u32,
    /// log2 of the LZ77 window, `16..=` the encoder's maximum.
    pub window_size_log2: u32,
    /// 0 = greedy, 1 = lazy match evaluation (only effective with 4 MTF entries, MTF minimum
    /// length 2 and a non-zero lookup depth).
    pub optimization_level: u32,
    /// Minimum length of a regular pointer: 3 or 4.
    pub ptr_min_match_length: u32,
    /// Minimum length of an MTF pointer: 2 or 3.
    pub mtf_min_match_length: u32,
}

impl EncoderParams {
    /// Parameters and runtime flags (for [`Encoder::with_flags`]) that reproduce, byte for
    /// byte apart from the random session signature, the XPress9 stream of a `DataModel`
    /// saved by Power BI Desktop 2.158.1304.0 (observed on a local sample; see the crate
    /// README): 4 MiB window, 4 MTF entries, lookup depth 3, lazy evaluation, pointer minimum
    /// 4, MTF minimum 2, SSE2 hash insertion.
    pub fn power_bi_desktop() -> (EncoderParams, u32) {
        (
            EncoderParams {
                max_stream_length: 0,
                mtf_entry_count: 4,
                lookup_depth: 3,
                window_size_log2: 22,
                optimization_level: 1,
                ptr_min_match_length: 4,
                mtf_min_match_length: 2,
            },
            FLAG_USE_SSE2,
        )
    }
}

impl Default for EncoderParams {
    /// The parameters used by the open-source XPress9 wrappers that produce PBIX-compatible
    /// streams (xpress9-python's `Compress`): 4 MTF entries, depth 9, 4 MiB window, greedy,
    /// pointer minimum 4, MTF minimum 2.
    fn default() -> Self {
        EncoderParams {
            max_stream_length: 0,
            mtf_entry_count: 4,
            lookup_depth: 9,
            window_size_log2: 22,
            optimization_level: 0,
            ptr_min_match_length: 4,
            mtf_min_match_length: 2,
        }
    }
}

/// `LZ77_MTF_STATE` (encoder side: negative offsets).
#[derive(Debug, Clone, Copy)]
struct MtfState {
    last_ptr: i64,
    offset: [i64; LZ77_MAX_MTF],
}

impl Default for MtfState {
    fn default() -> Self {
        MtfState {
            last_ptr: 0,
            offset: [-1; LZ77_MAX_MTF],
        }
    }
}

/// `s_iMaxOffsetByLength`: short matches are only taken when they are close enough.
const S_MAX_OFFSET_BY_LENGTH: [i64; 6] = [0, 0, -1 << 6, -1 << 10, -1 << 13, -1 << 16];

/// Extra zeroed bytes after the data buffer; upstream's match finder may peek a couple of
/// bytes past the end of the data (into stale buffer contents).
const DATA_SLACK: usize = 16;

/// An XPress9 encoder (`XPRESS9_ENCODER`).
pub struct Encoder {
    max_window_size_log2: u32,
    runtime_flags: u32,
    session_active: bool,
    params: EncoderParams,
    deep_lookup: bool,
    lazy: bool,

    // m_EncodeData
    data: Vec<u8>,
    data_size: usize,
    window_size: usize,
    encode_position: usize,
    hash_insert_position: usize,
    mtf: MtfState,

    // m_HashTable, m_uNext
    hash_table: Vec<u32>,
    hash_table_size_current: usize,
    next: Vec<u32>,

    // m_Pass2.m_Ir
    ir: Vec<u16>,
    ir_buffer_size: usize,
    ir_mtf: MtfState,

    // m_HuffmanStat
    short_symbol_count: Vec<u32>,
    long_length_count: Vec<u32>,
    stored_bit_count: u64,

    // m_Pass2 masks (kept across blocks, as upstream)
    short_symbol_mask: Vec<u32>,
    long_length_mask: Vec<u32>,
    scratch: HuffmanScratch,

    block_orig_size: u64,
    session_signature: u32,
    block_index: u32,
}

impl Encoder {
    /// `Xpress9EncoderCreate` with no runtime flags. Memory use is about
    /// `9 * 2^max_window_size_log2` bytes.
    pub fn new(max_window_size_log2: u32) -> Result<Encoder, Error> {
        Encoder::with_flags(max_window_size_log2, 0)
    }

    /// `Xpress9EncoderCreate` with `Xpress9Flag_*` runtime flags (see [`FLAG_USE_SSE2`]).
    pub fn with_flags(max_window_size_log2: u32, flags: u32) -> Result<Encoder, Error> {
        if !(WINDOW_SIZE_LOG2_MIN..=WINDOW_SIZE_LOG2_MAX).contains(&max_window_size_log2) {
            return Err(Error::BadArgument("max window size log2 out of range"));
        }
        let data_buffer_size = 1usize << max_window_size_log2;
        Ok(Encoder {
            max_window_size_log2,
            runtime_flags: flags,
            session_active: false,
            params: EncoderParams::default(),
            deep_lookup: false,
            lazy: false,
            data: vec![0; data_buffer_size + DATA_SLACK],
            data_size: 0,
            window_size: 0,
            encode_position: 0,
            hash_insert_position: 0,
            mtf: MtfState::default(),
            hash_table: vec![0; data_buffer_size >> 1],
            hash_table_size_current: 0,
            next: vec![0; data_buffer_size],
            ir: Vec::new(),
            ir_buffer_size: data_buffer_size * 2,
            ir_mtf: MtfState::default(),
            short_symbol_count: vec![0; LZ77_SHORT_SYMBOL_ALPHABET_SIZE],
            long_length_count: vec![0; LZ77_LONG_LENGTH_ALPHABET_SIZE],
            stored_bit_count: 0,
            short_symbol_mask: vec![0; LZ77_SHORT_SYMBOL_ALPHABET_SIZE],
            long_length_mask: vec![0; LZ77_LONG_LENGTH_ALPHABET_SIZE],
            scratch: HuffmanScratch::default(),
            block_orig_size: 0,
            session_signature: 0,
            block_index: 0,
        })
    }

    /// `Xpress9EncoderVerifyParameters` + `Xpress9GetLz77Procs`.
    fn verify_parameters(&self, p: &EncoderParams) -> Result<(), Error> {
        if !matches!(p.mtf_entry_count, 0 | 2 | 4) {
            return Err(Error::BadArgument("MTF entry count must be 0, 2 or 4"));
        }
        if p.lookup_depth > 9 && p.optimization_level == 0 {
            return Err(Error::BadArgument("lookup depth must be in 0..=9"));
        }
        if p.window_size_log2 < WINDOW_SIZE_LOG2_MIN || p.window_size_log2 > self.max_window_size_log2 {
            return Err(Error::BadArgument("window size log2 out of range"));
        }
        if p.optimization_level > 1 {
            return Err(Error::BadArgument("optimization level must be 0 or 1"));
        }
        if !(3..=4).contains(&p.ptr_min_match_length) {
            return Err(Error::BadArgument("pointer minimum match length must be 3 or 4"));
        }
        if p.mtf_min_match_length < 2 || p.mtf_min_match_length > p.ptr_min_match_length {
            return Err(Error::BadArgument("MTF minimum match length out of range"));
        }
        // Xpress9GetLz77Procs only instantiates MTF minimum lengths 2 and 3
        if p.mtf_min_match_length > 3 {
            return Err(Error::BadArgument("MTF minimum match length must be 2 or 3"));
        }
        Ok(())
    }

    /// `Xpress9EncoderStartSession (fForceReset = TRUE)`.
    ///
    /// `session_seed` replaces upstream's `__rdtsc()` value: the session signature written in
    /// every block header is `Xpress9Crc32 (seed as 8 little-endian bytes)`.
    pub fn start_session(&mut self, params: &EncoderParams, session_seed: u64) -> Result<(), Error> {
        self.verify_parameters(params)?;
        let p = *params;
        self.deep_lookup = p.lookup_depth != 0;
        self.lazy = p.optimization_level > 0
            && p.mtf_entry_count == 4
            && p.lookup_depth != 0
            && p.mtf_min_match_length == 2;

        self.ir.clear();
        self.data_size = 0;
        self.encode_position = 0;
        self.hash_insert_position = 0;

        let mut current = p;
        let max_len = u64::from(current.max_stream_length);
        if max_len != 0 && max_len < (1u64 << current.window_size_log2) {
            // if the stream is shorter than the window, shrink the window (to at least 512)
            let mut msb = get_msb(max_len)?;
            if max_len > (1u64 << msb) {
                msb += 1;
            }
            current.window_size_log2 = msb.max(9);
        }

        let mut msb = if max_len == 0 || max_len >= (1u64 << current.window_size_log2) {
            current.window_size_log2
        } else {
            let mut m = get_msb(max_len)?;
            if max_len > (1u64 << m) {
                m += 1;
            }
            m.max(9)
        };
        self.window_size = 1usize << msb;
        self.mtf = MtfState::default();

        msb -= 1;
        match current.lookup_depth {
            0 | 1 => msb = msb.min(12),
            2 => msb = msb.min(18),
            _ => {}
        }
        self.hash_table_size_current = 1usize << msb;
        self.hash_table
            .get_mut(..self.hash_table_size_current)
            .ok_or(Error::Internal("hash table size"))?
            .fill(0);

        self.params = current;
        self.block_orig_size = 0;
        self.session_signature = xpress9_crc32(&session_seed.to_le_bytes(), 0);
        self.block_index = 0;
        self.session_active = true;
        Ok(())
    }

    /// bicraft addition (not in upstream): preload `history` as the data that precedes the
    /// session's first byte, without encoding it. The match finder may then refer back into
    /// `history`, so the blocks only decode after a decoder session that has already produced
    /// those bytes (the case when independently compressed chunks of one stream continue a
    /// single decoder session, see [`crate::datamodel::CompressionMode::Balanced`]).
    ///
    /// Must be called right after [`Encoder::start_session`]. `history` is cut to its last
    /// `window / 2` bytes so the input that follows still has room in the window.
    pub fn prime(&mut self, history: &[u8]) -> Result<(), Error> {
        if !self.session_active || self.data_size != 0 || !self.ir.is_empty() {
            return Err(Error::BadArgument("prime must directly follow start_session"));
        }
        let keep = history.len().min(self.window_size >> 1);
        let tail = history
            .get(history.len() - keep..)
            .ok_or(Error::Internal("history range"))?;
        self.data
            .get_mut(..keep)
            .ok_or(Error::Internal("data buffer range"))?
            .copy_from_slice(tail);
        self.data_size = keep;
        self.insert()?;
        self.encode_position = keep;
        Ok(())
    }

    /// Attach `input`, compress it, fetch every finished block, detach
    /// (`Xpress9EncoderAttach` / `Compress` / `FetchCompressedData` / `Detach`).
    ///
    /// With `flush` the data buffered internally is encoded too, so the returned bytes decode
    /// to everything passed so far in the session. Without `flush` some input may stay
    /// buffered until a later call. Each call appends whole blocks only.
    pub fn compress(&mut self, input: &[u8], flush: bool) -> Result<Vec<u8>, Error> {
        if !self.session_active {
            return Err(Error::BadArgument("no encoder session; call start_session first"));
        }
        if input.is_empty() && !flush {
            return Err(Error::BadArgument("empty input requires flush"));
        }
        let result = self.compress_inner(input, flush);
        if result.is_err() {
            self.session_active = false;
        }
        result
    }

    fn compress_inner(&mut self, input: &[u8], flush: bool) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        let mut processed = 0usize;
        loop {
            // Xpress9EncoderCompress
            let mut flushing = false;
            while input.len() > processed {
                let user_bytes_available = input.len() - processed;

                // remaining IR capacity: room for each incoming byte plus the unhashed tail
                let mut ir_bytes_available = self.ir_buffer_size.saturating_sub(self.ir.len() * 2) / 3;
                if ir_bytes_available < 512 {
                    flushing = true;
                    break;
                }
                ir_bytes_available -= 256;

                let mut window_bytes_available = self
                    .window_size
                    .checked_sub(self.data_size)
                    .ok_or(Error::Internal("window overflow"))?;
                if window_bytes_available < 256 && window_bytes_available < user_bytes_available {
                    self.shift_window()?;
                    window_bytes_available = self.window_size - self.data_size;
                }

                let bytes_to_copy = ir_bytes_available
                    .min(window_bytes_available)
                    .min(user_bytes_available);
                if bytes_to_copy == 0 {
                    return Err(Error::Internal("no progress"));
                }
                let src = input
                    .get(processed..processed + bytes_to_copy)
                    .ok_or(Error::Internal("input range"))?;
                self.data
                    .get_mut(self.data_size..self.data_size + bytes_to_copy)
                    .ok_or(Error::Internal("data buffer range"))?
                    .copy_from_slice(src);
                self.data_size += bytes_to_copy;
                processed += bytes_to_copy;

                self.insert()?;

                if self.ir.is_empty() {
                    // first write to the IR buffer: clear statistics
                    self.short_symbol_count.fill(0);
                    self.long_length_count.fill(0);
                    self.stored_bit_count = 0;
                    self.ir_mtf = self.mtf;
                }

                let before = self.encode_position;
                self.pass1()?;
                self.block_orig_size += (self.encode_position - before) as u64;
            }

            if !flushing && flush {
                // encode the last few symbols that were not hashed as literals
                let mut position = self.encode_position;
                if self.data_size >= 8 + position {
                    return Err(Error::Internal("too much unencoded data at flush"));
                }
                while position < self.data_size {
                    self.encode_lit(position)?;
                    position += 1;
                    self.mtf.last_ptr = 0;
                    self.block_orig_size += 1;
                }
                self.encode_position = position;
                if !self.ir.is_empty() {
                    flushing = true;
                }
            }

            if !flushing {
                break;
            }
            self.flush_block(&mut out)?;
        }
        Ok(out)
    }

    /// `ShiftWindow`: drop the older half of the window.
    fn shift_window(&mut self) -> Result<(), Error> {
        let keep_size = self.window_size >> 1;
        let remove_size = self.window_size - keep_size;

        // ShiftValues on m_uNext and on the hash table
        for i in 0..keep_size {
            let v = self
                .next
                .get(i + remove_size)
                .copied()
                .ok_or(Error::Internal("next table range"))?;
            *self.next.get_mut(i).ok_or(Error::Internal("next table range"))? =
                (v as usize).saturating_sub(remove_size) as u32;
        }
        for v in self
            .hash_table
            .get_mut(..self.hash_table_size_current)
            .ok_or(Error::Internal("hash range"))?
        {
            *v = (*v as usize).saturating_sub(remove_size) as u32;
        }
        self.data.copy_within(remove_size..remove_size + keep_size, 0);

        let err = || Error::Internal("window shift underflow");
        self.encode_position = self.encode_position.checked_sub(remove_size).ok_or_else(err)?;
        self.hash_insert_position = self
            .hash_insert_position
            .checked_sub(remove_size)
            .ok_or_else(err)?;
        self.data_size = self.data_size.checked_sub(remove_size).ok_or_else(err)?;
        Ok(())
    }

    #[inline]
    fn byte(&self, i: usize) -> u8 {
        self.data.get(i).copied().unwrap_or(0)
    }

    /// Little-endian `u16` at `i`; bytes past the buffer read as 0.
    #[inline]
    fn read_u16(&self, i: usize) -> u16 {
        match self.data.get(i..).and_then(|s| s.first_chunk::<2>()) {
            Some(w) => u16::from_le_bytes(*w),
            None => u16::from(self.byte(i)) | (u16::from(self.byte(i.wrapping_add(1))) << 8),
        }
    }

    /// Little-endian `u32` at `i`; bytes past the buffer read as 0.
    #[inline]
    fn read_u32(&self, i: usize) -> u32 {
        match self.data.get(i..).and_then(|s| s.first_chunk::<4>()) {
            Some(w) => u32::from_le_bytes(*w),
            None => u32::from(self.read_u16(i)) | (u32::from(self.read_u16(i.wrapping_add(2))) << 16),
        }
    }

    /// bicraft addition (same result as the byte loops of upstream's match finder): the first
    /// `p >= from` where `data[p] != data[p - delta]`, or `data_size` if there is none before
    /// it. Compares eight bytes at a time. `delta` must be in `1..=from`.
    #[inline]
    fn extend_match(&self, from: usize, delta: usize) -> usize {
        let end = self.data_size;
        let mut p = from;
        while p < end {
            let a = self.data.get(p..).and_then(|s| s.first_chunk::<8>());
            let q = p.wrapping_sub(delta);
            let b = self.data.get(q..).and_then(|s| s.first_chunk::<8>());
            match (a, b) {
                (Some(a), Some(b)) => {
                    let x = u64::from_le_bytes(*a) ^ u64::from_le_bytes(*b);
                    if x != 0 {
                        let at = p + (x.trailing_zeros() / 8) as usize;
                        return at.min(end);
                    }
                    p += 8;
                }
                _ => {
                    if self.byte(p) != self.byte(q) {
                        return p;
                    }
                    p += 1;
                }
            }
        }
        end
    }

    #[inline]
    fn next_at(&self, i: usize) -> Result<usize, Error> {
        self.next
            .get(i)
            .map(|&v| v as usize)
            .ok_or(Error::Internal("next table index"))
    }

    #[inline]
    fn set_next(&mut self, i: usize, v: usize) -> Result<(), Error> {
        *self.next.get_mut(i).ok_or(Error::Internal("next table index"))? = v as u32;
        Ok(())
    }

    fn link(&mut self, position: usize, hash: usize) -> Result<(), Error> {
        let slot = self
            .hash_table
            .get_mut(hash)
            .ok_or(Error::Internal("hash index"))?;
        let head = *slot;
        *slot = position as u32;
        self.set_next(position, head as usize)
    }

    /// `Xpress9Lz77EncInsert_SSE2`: scalar hashing (64-bit arithmetic) until the data is
    /// 16-byte aligned, then 16 positions at a time with 32-bit lanes, then scalar again.
    fn insert_sse2(&mut self) -> Result<(), Error> {
        let mut position = self.hash_insert_position;
        let mut last_position = self.data_size;
        if last_position >= 4 {
            let hash_mask = self.hash_table_size_current - 1;
            let scalar = |v: u32| -> usize {
                let v = u64::from(v);
                let mut h = (v ^ 0xdead_beef).wrapping_add(v >> 5);
                h ^= h >> 11;
                h ^= h >> 17;
                h as usize
            };
            let lane = |v: u32| -> usize {
                let mut h = (v ^ 0xdead_beef).wrapping_add(v >> 5);
                h ^= h >> 11;
                h ^= h >> 17;
                h as usize
            };
            last_position -= 4;
            while position < last_position && position & 15 != 0 {
                let h = scalar(self.read_u32(position)) & hash_mask;
                self.link(position, h)?;
                position += 1;
            }
            if last_position >= 32 && last_position - 32 >= position {
                last_position -= 32;
                loop {
                    for k in 0..16 {
                        let h = lane(self.read_u32(position + k)) & hash_mask;
                        self.link(position + k, h)?;
                    }
                    position += 16;
                    if position >= last_position {
                        break;
                    }
                }
                last_position += 32;
            }
            while position < last_position {
                let h = scalar(self.read_u32(position)) & hash_mask;
                self.link(position, h)?;
                position += 1;
            }
            last_position += 4;
            self.hash_insert_position = position;
        }
        while position < last_position {
            self.set_next(position, 0)?;
            position += 1;
        }
        Ok(())
    }

    /// `Xpress9Lz77EncInsert`: add the new positions to the hash chains.
    fn insert(&mut self) -> Result<(), Error> {
        if self.params.ptr_min_match_length == 4 && self.runtime_flags & FLAG_USE_SSE2 != 0 {
            return self.insert_sse2();
        }
        let mut position = self.hash_insert_position;
        let last_position = self.data_size;
        if last_position >= 4 {
            let hash_mask = self.hash_table_size_current - 1;
            let stop = last_position - 4;
            while position < stop {
                let hash = (if self.params.ptr_min_match_length == 4 {
                    let v = u64::from(self.read_u32(position));
                    let mut h = (v ^ 0xdead_beef).wrapping_add(v >> 5);
                    h ^= h >> 11;
                    h as usize
                } else {
                    let z = |b: u8, k: usize| {
                        ZOBRIST_TABLE
                            .get(b as usize)
                            .and_then(|row| row.get(k))
                            .copied()
                            .unwrap_or(0)
                    };
                    (z(self.byte(position), 0)
                        ^ z(self.byte(position + 1), 1)
                        ^ z(self.byte(position + 2), 2)) as usize
                }) & hash_mask;
                let slot = self
                    .hash_table
                    .get_mut(hash)
                    .ok_or(Error::Internal("hash index"))?;
                let head = *slot;
                *slot = position as u32;
                self.set_next(position, head as usize)?;
                position += 1;
            }
            self.hash_insert_position = position;
        }
        // the last few bytes stay unhashed
        while position < last_position {
            self.set_next(position, 0)?;
            position += 1;
        }
        Ok(())
    }

    // ------------------------------------------------------------- IR emission (ENCODE_*)

    fn count_short(&mut self, symbol: usize) -> Result<(), Error> {
        let c = self
            .short_symbol_count
            .get_mut(symbol)
            .ok_or(Error::Internal("short symbol"))?;
        *c = c.wrapping_add(1);
        Ok(())
    }

    fn count_long(&mut self, symbol: usize) -> Result<(), Error> {
        let c = self
            .long_length_count
            .get_mut(symbol)
            .ok_or(Error::Internal("long length symbol"))?;
        *c = c.wrapping_add(1);
        Ok(())
    }

    fn push_value(&mut self, value: usize, msb: u32) {
        if msb > 16 {
            self.ir.push(value as u16);
            self.ir.push((value >> 16) as u16);
        } else {
            self.ir.push(value as u16);
        }
    }

    /// `ENCODE_LIT`.
    fn encode_lit(&mut self, position: usize) -> Result<(), Error> {
        let symbol = self
            .data
            .get(position)
            .copied()
            .ok_or(Error::Internal("literal position"))? as usize;
        self.ir.push(symbol as u16);
        self.count_short(symbol)
    }

    /// The length part shared by `ENCODE_PTR` and `ENCODE_MTF`.
    fn encode_length(
        &mut self,
        mut symbol: usize,
        best_length: usize,
        min_match_length: usize,
    ) -> Result<(), Error> {
        let mut length = best_length
            .checked_sub(min_match_length)
            .ok_or(Error::Internal("match too short"))?;
        if length < LZ77_MAX_SHORT_LENGTH - 1 {
            symbol += length;
            self.ir.push(symbol as u16);
            self.count_short(symbol)?;
        } else {
            symbol += LZ77_MAX_SHORT_LENGTH - 1;
            length -= LZ77_MAX_SHORT_LENGTH - 1;
            self.ir.push(symbol as u16);
            self.count_short(symbol)?;
            if length < LZ77_MAX_LONG_LENGTH {
                self.ir.push(length as u16);
                self.count_long(length)?;
            } else {
                length -= LZ77_MAX_LONG_LENGTH - 1;
                let msb = get_msb(length as u64)?;
                length -= 1usize << msb;
                self.stored_bit_count += u64::from(msb);
                let long_symbol = msb as usize + LZ77_MAX_LONG_LENGTH;
                self.ir.push(long_symbol as u16);
                self.count_long(long_symbol)?;
                self.push_value(length, msb);
            }
        }
        Ok(())
    }

    /// `ENCODE_PTR`.
    fn encode_ptr(
        &mut self,
        best_length: usize,
        min_match_length: usize,
        best_offset: i64,
    ) -> Result<(), Error> {
        if best_offset >= 0 {
            return Err(Error::Internal("pointer offset must be negative"));
        }
        let mut offset = best_offset.unsigned_abs() as usize;
        let msb = get_msb(offset as u64)?;
        offset -= 1usize << msb;
        self.stored_bit_count += u64::from(msb);
        let symbol =
            (msb as usize + (256 >> LZ77_MAX_SHORT_LENGTH_LOG) + self.params.mtf_entry_count as usize)
                << LZ77_MAX_SHORT_LENGTH_LOG;
        self.encode_length(symbol, best_length, min_match_length)?;
        self.push_value(offset, msb);
        Ok(())
    }

    /// `ENCODE_MTF`.
    fn encode_mtf(
        &mut self,
        best_length: usize,
        min_match_length: usize,
        mtf_index: usize,
    ) -> Result<(), Error> {
        let symbol = (mtf_index + (256 >> LZ77_MAX_SHORT_LENGTH_LOG)) << LZ77_MAX_SHORT_LENGTH_LOG;
        self.encode_length(symbol, best_length, min_match_length)
    }

    // ----------------------------------------------------------------------- match finding

    /// The MTF test of `CHECK_MTF` and friends: length of the match at `offset` (negative)
    /// from `position`, if at least the MTF minimum length.
    fn mtf_match(&self, position: usize, offset: i64) -> Option<usize> {
        let min = self.params.mtf_min_match_length as usize;
        if (position as i64) + offset <= 0 {
            return None;
        }
        let src = (position as i64 + offset) as usize;
        if self.byte(src) != self.byte(position)
            || self.byte(src + 1) != self.byte(position + 1)
            || (min > 2 && self.byte(src + 2) != self.byte(position + 2))
            || (min > 3 && self.byte(src + 3) != self.byte(position + 3))
        {
            return None;
        }
        let delta = position - src;
        let p = self.extend_match(position + min, delta).max(position + min);
        Some(p - position)
    }

    /// `Xpress9Lookup.i` (and its `.i2` / `.i3` copies): walk the hash chain from `position`
    /// looking for a match longer than `best_length`.
    fn lookup(
        &mut self,
        position: usize,
        best_length: &mut usize,
        best_offset: &mut i64,
        max_depth: usize,
    ) -> Result<(), Error> {
        let deep = self.deep_lookup;
        let mut depth = max_depth;
        let mut candidate = self.next_at(position)?;
        self.set_next(0, position)?; // 0 is the end of a hash chain: make it point at us

        loop {
            let base = best_length.wrapping_sub(1);
            let tail = self.read_u16(position.wrapping_add(base));
            let same = |s: &Self, c: usize| s.read_u16(c.wrapping_add(base)) == tail;

            let found: usize = 'inner: loop {
                if deep {
                    if depth == 0 {
                        return Ok(());
                    }
                    depth -= 1;
                }
                let mut c = candidate;
                for _ in 0..4 {
                    let c2 = self.next_at(c)?;
                    if same(self, c) {
                        break 'inner c;
                    }
                    let c_next = self.next_at(c2)?;
                    if same(self, c2) {
                        break 'inner c2;
                    }
                    c = c_next;
                }
                if !deep {
                    return Ok(());
                }
                candidate = c;
            };
            let c = found;

            if c >= position {
                return Ok(());
            }
            let offset = c as i64 - position as i64;
            let delta = position - c;
            let p = self.extend_match(position, delta);
            if p >= self.data_size {
                *best_length = p - position;
                *best_offset = offset;
                return Ok(());
            }
            let length = p - position;
            if length > *best_length
                && (length > S_MAX_OFFSET_BY_LENGTH.len() - 1
                    || offset > S_MAX_OFFSET_BY_LENGTH.get(length).copied().unwrap_or(0))
            {
                *best_length = length;
                *best_offset = offset;
            }

            if !deep {
                return Ok(());
            }
            candidate = self.next_at(c)?;
        }
    }

    /// `UPDATE_MTF (iIndex, iOffset)`.
    fn update_mtf(off: &mut [i64; LZ77_MAX_MTF], mtf_count: usize, index: usize, offset: i64) {
        let [o0, o1, o2, o3] = off;
        if mtf_count == 4 {
            if index >= 3 {
                *o3 = *o2;
            }
            if index >= 2 {
                *o2 = *o1;
            }
            *o1 = *o0;
            *o0 = offset;
        } else if mtf_count == 2 {
            *o1 = *o0;
            *o0 = offset;
        }
    }

    /// `Xpress9Lz77EncPass1`: turn the hashed data into the intermediate representation.
    fn pass1(&mut self) -> Result<(), Error> {
        let mtf_count = self.params.mtf_entry_count as usize;
        let ptr_min = self.params.ptr_min_match_length as usize;
        let mtf_min = self.params.mtf_min_match_length as usize;
        let max_depth = if self.deep_lookup {
            self.params.lookup_depth as usize + 1
        } else {
            0
        };
        let stop = self.hash_insert_position;

        let mut position = self.encode_position;
        let mut last_ptr = self.mtf.last_ptr;
        let mut off = self.mtf.offset;

        'outer: loop {
            // (length, encode index, update index, offset) of an MTF match to emit
            let mut mtf_hit: Option<(usize, usize, usize, i64)> = None;

            if mtf_count >= 2 {
                if last_ptr == 0
                    && let Some(len) = self.mtf_match(position, off[0])
                {
                    mtf_hit = Some((len, 0, 0, off[0]));
                }
                for (k, &o) in off.iter().enumerate().take(mtf_count).skip(1) {
                    if mtf_hit.is_some() {
                        break;
                    }
                    if let Some(len) = self.mtf_match(position, o) {
                        mtf_hit = Some((len, (k as i64 + last_ptr) as usize, k, o));
                    }
                }
            }

            let mut literal = mtf_hit.is_none() && self.next_at(position)? == 0;

            if mtf_hit.is_none() && !literal {
                self.set_next(0, position)?;
                let mut best_length = ptr_min - 1;
                let mut best_offset: i64 = 0;
                self.lookup(position, &mut best_length, &mut best_offset, max_depth)?;

                if best_length >= ptr_min {
                    if self.lazy
                        && position + 2 < stop
                        && self.next_at(position + 1)? != 0
                        && let Some(hit) = self.lazy_evaluation(
                            &mut position,
                            &mut best_length,
                            &mut best_offset,
                            max_depth,
                            &off,
                        )?
                    {
                        mtf_hit = Some(hit);
                    }
                    if mtf_hit.is_none() {
                        self.encode_ptr(best_length, ptr_min, best_offset)?;
                        if mtf_count >= 2 {
                            Self::update_mtf(&mut off, mtf_count, mtf_count - 1, best_offset);
                        }
                        last_ptr = -1;
                        position += best_length;
                    }
                } else {
                    literal = true;
                }
            }

            if let Some((len, encode_index, update_index, offset)) = mtf_hit {
                // EncodeMtfPtr
                self.encode_mtf(len, mtf_min, encode_index)?;
                if update_index != 0 {
                    Self::update_mtf(&mut off, mtf_count, update_index, offset);
                }
                last_ptr = -1;
                position += len;
            } else if literal {
                last_ptr = 0;
                loop {
                    self.encode_lit(position)?;
                    position += 1;
                    if position >= stop {
                        break 'outer;
                    }
                    let candidate = self.next_at(position)?;
                    let matches = candidate != 0
                        && if ptr_min == 4 {
                            self.read_u32(position) == self.read_u32(candidate)
                        } else {
                            self.byte(position) == self.byte(candidate)
                                && self.byte(position + 1) == self.byte(candidate + 1)
                                && self.byte(position + 2) == self.byte(candidate + 2)
                        };
                    if matches {
                        break;
                    }
                }
            }

            if position >= stop {
                break;
            }
        }

        self.encode_position = position;
        self.mtf.last_ptr = last_ptr;
        self.mtf.offset = off;
        Ok(())
    }

    /// The `LAZY_MATCH_EVALUATION` block of pass 1: look one and two positions ahead for a
    /// better match. Emits the literals it skips. Returns an MTF match to emit instead of the
    /// pointer, if one was taken; otherwise leaves the pointer in `best_*` at `*position`.
    fn lazy_evaluation(
        &mut self,
        position: &mut usize,
        best_length: &mut usize,
        best_offset: &mut i64,
        max_depth: usize,
        off: &[i64; LZ77_MAX_MTF],
    ) -> Result<Option<(usize, usize, usize, i64)>, Error> {
        let saved_best_length = *best_length;
        let saved_best_offset = *best_offset;
        let max_depth2 = max_depth / 2;

        *position += 1;
        // take any MTF match at position + 1 that is not much shorter (a literal goes first)
        for (k, &o) in off.iter().enumerate() {
            if let Some(len) = self.mtf_match(*position, o)
                && len >= saved_best_length.wrapping_sub(3)
            {
                self.encode_lit(*position - 1)?;
                return Ok(Some((len, k, k, o)));
            }
        }
        // xpress9lookup.i2; the search may trash best_* even without a longer match
        self.lookup(*position, best_length, best_offset, max_depth2)?;
        *position -= 1;

        if *best_length > saved_best_length {
            if self.next_at(*position + 2)? != 0 {
                let saved_best_length2 = *best_length;
                let saved_best_offset2 = *best_offset;
                let max_depth3 = max_depth / 4;
                *position += 2;
                for (k, &o) in off.iter().enumerate() {
                    if let Some(len) = self.mtf_match(*position, o)
                        && len >= saved_best_length2.wrapping_sub(3)
                    {
                        self.encode_lit(*position - 2)?;
                        self.encode_lit(*position - 1)?;
                        return Ok(Some((len, k, k, o)));
                    }
                }
                // xpress9lookup.i3
                self.lookup(*position, best_length, best_offset, max_depth3)?;
                *position -= 2;
                if *best_length > saved_best_length2 {
                    // an even longer match at position + 2
                    self.encode_lit(*position)?;
                    *position += 1;
                    self.encode_lit(*position)?;
                    *position += 1;
                } else {
                    // the longest match is at position + 1
                    self.encode_lit(*position)?;
                    *position += 1;
                    *best_offset = saved_best_offset2;
                    *best_length = saved_best_length2;
                }
            } else {
                self.encode_lit(*position)?;
                *position += 1;
            }
        } else {
            let saved_best_length2 = saved_best_length;
            *position += 2;
            for (k, &o) in off.iter().enumerate() {
                if let Some(len) = self.mtf_match(*position, o)
                    && len >= saved_best_length2.wrapping_sub(3)
                {
                    self.encode_lit(*position - 2)?;
                    self.encode_lit(*position - 1)?;
                    return Ok(Some((len, k, k, o)));
                }
            }
            *position -= 2;
            *best_offset = saved_best_offset;
            *best_length = saved_best_length;
        }
        Ok(None)
    }

    // ---------------------------------------------------------------------- block output

    /// `StartFlushing` + `Xpress9Lz77EncPass2` + `Xpress9EncoderFetchCompressedData`: emit
    /// the buffered IR as one block.
    fn flush_block(&mut self, out: &mut Vec<u8>) -> Result<(), Error> {
        if self.ir.is_empty() {
            return Err(Error::Internal("flushing an empty block"));
        }
        let mtf_count = self.params.mtf_entry_count as usize;
        let mut w = BitWriter::new();
        // leave room for the header
        for _ in 0..8 {
            w.write(0, 32);
        }
        if mtf_count > 0 {
            w.write((self.ir_mtf.last_ptr & 1) as u64, 1);
            for &o in self.ir_mtf.offset.iter().take(mtf_count) {
                if o >= 0 {
                    return Err(Error::Internal("MTF offset must be negative"));
                }
                let mut offset = o.unsigned_abs();
                let msb = get_msb(offset)?;
                offset ^= 1u64 << msb;
                w.write(u64::from(msb), 5);
                w.write(offset, msb);
            }
        }
        let header_cost = w.bit_position() as usize;

        let short_symbol_cost = xpress9_huffman_create_and_encode_table(
            &mut w,
            &self.short_symbol_count,
            HUFFMAN_MAX_CODEWORD_LENGTH,
            &mut self.short_symbol_mask,
            LZ77_MAX_SHORT_LENGTH,
            &mut self.scratch,
        )?;
        let long_length_cost = xpress9_huffman_create_and_encode_table(
            &mut w,
            &self.long_length_count,
            HUFFMAN_MAX_CODEWORD_LENGTH,
            &mut self.long_length_mask,
            LZ77_MAX_SHORT_LENGTH,
            &mut self.scratch,
        )?;
        let encoded_huffman_tables_bits = w.bit_position() as usize - 8 * 32;

        let computed_encoded_size_bits =
            header_cost + short_symbol_cost + long_length_cost + self.stored_bit_count as usize;

        self.pass2(&mut w)?;
        if w.bit_position() != computed_encoded_size_bits as u64 {
            return Err(Error::Internal("computed and actual block sizes differ"));
        }

        let p = &self.params;
        let mut flags = (encoded_huffman_tables_bits & 0x1fff) as u32;
        flags |= (p.window_size_log2.wrapping_sub(16) & 7) << 13;
        flags |= (p.mtf_entry_count >> 1) << 16;
        flags |= (p.ptr_min_match_length.wrapping_sub(3) & 1) << 18;
        flags |= (p.mtf_min_match_length.wrapping_sub(2) & 1) << 19;

        let mut bytes = w.finish();
        let words = [
            XPRESS9_MAGIC,
            self.block_orig_size as u32,
            computed_encoded_size_bits as u32,
            flags,
            0,
            self.session_signature,
            self.block_index,
        ];
        for (i, &v) in words.iter().enumerate() {
            BitWriter::patch_u32(&mut bytes, i, v)?;
        }
        let crc = xpress9_crc32(bytes.get(..28).ok_or(Error::Internal("header"))?, 0);
        BitWriter::patch_u32(&mut bytes, 7, crc)?;
        self.block_index = self.block_index.wrapping_add(1);

        out.extend_from_slice(&bytes);
        self.ir.clear();
        self.block_orig_size = 0;
        Ok(())
    }

    /// `Xpress9Lz77EncPass2`: Huffman-code the intermediate representation.
    fn pass2(&self, w: &mut BitWriter) -> Result<(), Error> {
        let mtf_count = self.params.mtf_entry_count as usize;
        let ir = &self.ir;
        let mut i = 0usize;
        let take = |i: &mut usize, n: usize| -> Result<usize, Error> {
            let lo = ir.get(*i).copied().ok_or(Error::Internal("IR underflow"))? as usize;
            let v = if n == 2 {
                lo | ((ir.get(*i + 1).copied().ok_or(Error::Internal("IR underflow"))? as usize) << 16)
            } else {
                lo
            };
            *i += n;
            Ok(v)
        };
        let short = |s: usize| {
            self.short_symbol_mask
                .get(s)
                .copied()
                .ok_or(Error::Internal("short symbol"))
        };
        let long = |s: usize| {
            self.long_length_mask
                .get(s)
                .copied()
                .ok_or(Error::Internal("long symbol"))
        };

        while i < ir.len() {
            let symbol = take(&mut i, 1)?;
            w.write_symbol(short(symbol)?);
            if symbol >= 256 {
                // a pointer (explicit or MTF): encode the length
                let mut bits = symbol & (LZ77_MAX_SHORT_LENGTH - 1);
                if bits >= LZ77_MAX_SHORT_LENGTH - 1 {
                    bits = take(&mut i, 1)?;
                    w.write_symbol(long(bits)?);
                    if bits >= LZ77_MAX_LONG_LENGTH {
                        bits -= LZ77_MAX_LONG_LENGTH;
                        let value = take(&mut i, if bits > 16 { 2 } else { 1 })?;
                        w.write(value as u64, bits as u32);
                    }
                }
                if mtf_count == 0 || symbol >= 256 + (mtf_count << LZ77_MAX_SHORT_LENGTH_LOG) {
                    // encode the offset
                    let bits = (symbol >> LZ77_MAX_SHORT_LENGTH_LOG)
                        - ((256 >> LZ77_MAX_SHORT_LENGTH_LOG) + mtf_count);
                    let value = take(&mut i, if bits > 16 { 2 } else { 1 })?;
                    w.write(value as u64, bits as u32);
                }
            }
        }
        Ok(())
    }
}
