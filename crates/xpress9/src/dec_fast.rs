// The decode loop below re-implements the ported Microsoft XPress9 decoder (MIT) of
// `dec_lz77.rs` / `dec_huffman.rs` (dev/ese/src/_xpress9/Xpress9Lz77Dec.i@7030fe7407615160e54d152e4ef704eede2fdd7e)
// for whole blocks. Copyright (c) Microsoft Corporation. Licensed under the MIT License.
// See LICENSE-MIT-microsoft-xpress9 in this crate and THIRD_PARTY_NOTICES.md at the repository root.

//! The fast XPress9 decoder: whole blocks decoded straight into the output, and independent
//! groups of blocks ("jobs") decoded on several threads.
//!
//! The reference decoder ([`crate::Decoder`]) follows upstream's streaming design: a history
//! buffer of `1.5 * 2^window` bytes that is shifted as it fills, slices paced by the compressed
//! bytes at hand, and a copy of every decoded slice into the caller's vector. Here a block is
//! decoded in one pass into its final place, with a 64-bit bit buffer, table-driven symbol
//! lookups and word-sized match copies.
//!
//! **Equivalence.** Every check of the reference decoder is kept, and one is stricter: a match
//! may reach back at most `min(session position, 2^window)` bytes. Upstream accepts at least
//! that much (its buffer always holds the last `2^window` bytes, or everything since the
//! session started), and its encoder never reaches further. So whenever this decoder succeeds,
//! the reference decoder succeeds with the same bytes. Callers fall back to the reference
//! decoder on any error, so errors (and the rare stream that only the reference accepts) are
//! reported exactly as before.
//!
//! **Parallel decoding.** Blocks only depend on each other through the history (each block
//! header carries its Huffman tables and its MTF state, and a successful block never leaves a
//! pending match tail). A job that does not start its session is first decoded with the
//! history unknown ("deferred"): literals and matches whose source lies inside the job and is
//! already known are written; a match that reads the unknown history, or bytes that depend on
//! it, is recorded instead. Once the jobs before it are final, the recorded matches are
//! replayed in order. Decoding the symbols (the expensive part) thus runs in parallel, and only
//! the replay is sequential. A job that would record too many matches is decoded again, in
//! order, with the history known.

use crate::Error;
use crate::dec_huffman::{DecodeEntry, huffman_create_decode_tables, huffman_decode_length_table};
use crate::dec_lz77::{
    BLOCK_HEADER_SIZE, BlockHeader, LONG_TABLE_SIZE, LZ77_LONG_LENGTH_DECODE_ROOT_BITS,
    LZ77_LONG_LENGTH_DECODE_TAIL_BITS, LZ77_SHORT_SYMBOL_DECODE_ROOT_BITS,
    LZ77_SHORT_SYMBOL_DECODE_TAIL_BITS, SHORT_TABLE_SIZE,
};
use crate::misc::{
    BitReader, HUFFMAN_MAX_CODEWORD_LENGTH, LZ77_LONG_LENGTH_ALPHABET_SIZE, LZ77_MAX_LONG_LENGTH,
    LZ77_MAX_SHORT_LENGTH, LZ77_MAX_SHORT_LENGTH_LOG, LZ77_SHORT_SYMBOL_ALPHABET_SIZE,
};

const SHORT_ROOT_BITS: u32 = LZ77_SHORT_SYMBOL_DECODE_ROOT_BITS as u32;
const SHORT_TAIL_BITS: u32 = LZ77_SHORT_SYMBOL_DECODE_TAIL_BITS as u32;
const LONG_ROOT_BITS: u32 = LZ77_LONG_LENGTH_DECODE_ROOT_BITS as u32;
const LONG_TAIL_BITS: u32 = LZ77_LONG_LENGTH_DECODE_TAIL_BITS as u32;

/// Session parameters, fixed by the first block of a session (`Xpress9DecoderStartSession`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SessionParams {
    pub window_size_log2: u32,
    pub mtf_count: usize,
    pub ptr_min: usize,
    pub mtf_min: usize,
}

impl SessionParams {
    fn of(h: &BlockHeader) -> SessionParams {
        SessionParams {
            window_size_log2: h.window_size_log2(),
            mtf_count: h.mtf_entry_count() as usize,
            ptr_min: h.ptr_min_match_length() as usize,
            mtf_min: h.mtf_min_match_length() as usize,
        }
    }
}

/// One block of the input, header already validated.
pub(crate) struct BlockRef<'a> {
    /// The whole block, header included (`header.block_size()` bytes).
    pub bytes: &'a [u8],
    pub header: BlockHeader,
}

/// A run of consecutive blocks of one session, decoded as a unit.
#[derive(Debug, Clone)]
pub(crate) struct Job {
    pub blocks: std::ops::Range<usize>,
    /// Offset of the job's first byte in the whole output.
    pub out_start: usize,
    pub out_len: usize,
    /// Offset of the first byte of the job's session in the whole output.
    pub session_start: usize,
    pub params: SessionParams,
}

impl Job {
    /// Bytes of the session that precede the job.
    pub fn history_len(&self) -> usize {
        self.out_start.saturating_sub(self.session_start)
    }
}

/// The blocks of a whole input, checked, with their place in the output.
pub(crate) struct Plan<'a> {
    pub blocks: Vec<BlockRef<'a>>,
    /// Per block: (output offset, offset of its session's first byte, session parameters).
    places: Vec<(usize, usize, SessionParams)>,
    pub total: usize,
}

impl Plan<'_> {
    /// Groups consecutive blocks of one session into jobs of at least `target` bytes (a job
    /// ends after the block that reaches the target, or at a session boundary).
    pub fn jobs(&self, target: usize) -> Vec<Job> {
        let mut jobs: Vec<Job> = Vec::new();
        for (index, (block, &(out_start, session_start, params))) in
            self.blocks.iter().zip(&self.places).enumerate()
        {
            let len = block.header.original_size as usize;
            match jobs.last_mut() {
                Some(j) if j.session_start == session_start && j.out_len < target => {
                    j.blocks.end = index + 1;
                    j.out_len += len;
                }
                _ => jobs.push(Job {
                    blocks: index..index + 1,
                    out_start,
                    out_len: len,
                    session_start,
                    params,
                }),
            }
        }
        jobs
    }
}

/// One unit of the input as the framing presents it: `(session, payload, expected size)`.
/// Units of one session are consecutive; a new session index starts a new decoder session.
pub(crate) type Unit<'a> = (usize, &'a [u8], usize);

/// Parses and checks every block header the way the reference decoder would meet them (block
/// index sequence, session signature and parameters, window limit, exact tiling of each unit,
/// decoded sizes adding up to each unit's expected size). `None` when anything is off; the
/// caller then uses the reference decoder, which reports the precise error.
pub(crate) fn plan<'a>(units: &[Unit<'a>], max_window_log2: u32) -> Option<Plan<'a>> {
    plan_from(units, max_window_log2, PlanState::default()).map(|(p, _)| p)
}

/// Where the plan of one piece of an input stopped: the plan of the next piece goes on from it
/// (a session that continues keeps its signature, block numbering and first byte).
#[derive(Debug, Clone, Copy)]
pub(crate) struct PlanState {
    /// Bytes of output before the piece.
    pub out_pos: usize,
    current_session: usize,
    session: Option<(u32, u32, SessionParams)>,
    session_start: usize,
}

impl Default for PlanState {
    fn default() -> Self {
        PlanState {
            out_pos: 0,
            current_session: usize::MAX,
            session: None,
            session_start: 0,
        }
    }
}

/// [`plan`] of one piece of an input whose earlier pieces ended at `state`: output offsets count
/// from the start of the whole input. Returns the plan and where it ends.
pub(crate) fn plan_from<'a>(
    units: &[Unit<'a>],
    max_window_log2: u32,
    state: PlanState,
) -> Option<(Plan<'a>, PlanState)> {
    let mut blocks: Vec<BlockRef<'a>> = Vec::new();
    let mut places: Vec<(usize, usize, SessionParams)> = Vec::new();
    let mut out_pos: usize = state.out_pos;
    let mut current_session = state.current_session;
    // (signature, next block index, params) once the session's first block was seen
    let mut session: Option<(u32, u32, SessionParams)> = state.session;
    let mut session_start = state.session_start;

    for &(session_id, payload, expected) in units {
        if session_id != current_session {
            current_session = session_id;
            session = None;
            session_start = out_pos;
        }
        let mut at = 0usize;
        let mut produced = 0usize;
        while at < payload.len() {
            let rest = payload.get(at..)?;
            let header = BlockHeader::parse(rest).ok()?;
            let size = header.block_size();
            let bytes = rest.get(..size)?;
            let params = SessionParams::of(&header);
            match &mut session {
                None => {
                    if header.block_index != 0 || params.window_size_log2 > max_window_log2 {
                        return None;
                    }
                    session = Some((header.session_signature, 1, params));
                }
                Some((signature, next, p)) => {
                    if header.block_index != *next || header.session_signature != *signature || params != *p {
                        return None;
                    }
                    *next = next.wrapping_add(1);
                }
            }
            let orig = header.original_size as usize;
            produced = produced.checked_add(orig)?;
            if produced > expected {
                return None;
            }
            places.push((out_pos, session_start, params));
            out_pos = out_pos.checked_add(orig)?;
            blocks.push(BlockRef { bytes, header });
            at = at.checked_add(size).filter(|_| size > 0)?;
        }
        if produced != expected {
            return None;
        }
    }
    Some((
        Plan {
            blocks,
            places,
            total: out_pos,
        },
        PlanState {
            out_pos,
            current_session,
            session,
            session_start,
        },
    ))
}

// ------------------------------------------------------------------------------------ history

/// The bytes of a session that precede a job, possibly in several pieces (oldest first). Only
/// the last `2^window` bytes are ever read.
pub(crate) struct History<'s, 'a> {
    parts: &'s [&'a [u8]],
    len: usize,
}

impl<'s, 'a> History<'s, 'a> {
    pub fn new(parts: &'s [&'a [u8]]) -> History<'s, 'a> {
        History {
            parts,
            len: parts.iter().map(|p| p.len()).sum(),
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// Copies `dst.len()` bytes starting `back` bytes before the end of the history.
    fn copy_back(&self, back: usize, dst: &mut [u8]) -> Result<(), Error> {
        let bad = Error::Internal("history range");
        if dst.len() > back {
            return Err(bad);
        }
        let mut pos = self.len.checked_sub(back).ok_or(bad)?;
        // find the part holding `pos`, walking back from the newest
        let mut idx = self.parts.len();
        let mut part_start = self.len;
        while idx > 0 && part_start > pos {
            idx -= 1;
            part_start -= self.parts.get(idx).map_or(0, |p| p.len());
        }
        let mut written = 0usize;
        while written < dst.len() {
            let part = self.parts.get(idx).ok_or(Error::Internal("history range"))?;
            let from = pos - part_start;
            let n = (dst.len() - written).min(part.len().saturating_sub(from));
            let src = part.get(from..from + n).ok_or(Error::Internal("history range"))?;
            dst.get_mut(written..written + n)
                .ok_or(Error::Internal("history range"))?
                .copy_from_slice(src);
            written += n;
            pos += n;
            part_start += part.len();
            idx += 1;
        }
        Ok(())
    }
}

// ----------------------------------------------------------------------------------- bit input

/// LSB-first bit reader with a 64-bit buffer. Bits past the end of the data read as zero, like
/// [`BitReader`]; the caller checks the consumed count against the block's encoded size.
struct Bits<'a> {
    data: &'a [u8],
    /// Next byte not yet counted in `n` (may run past the end: virtual zero bytes).
    next: usize,
    /// Buffered bits; bits above `n` are either zero or the stream's own next bits.
    buf: u64,
    n: u32,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8], start_bit: u64) -> Bits<'a> {
        let mut b = Bits {
            data,
            next: usize::try_from(start_bit >> 3).unwrap_or(usize::MAX),
            buf: 0,
            n: 0,
        };
        b.ensure();
        b.consume((start_bit & 7) as u32);
        b
    }

    /// Makes at least 32 bits available.
    #[inline(always)]
    fn ensure(&mut self) {
        if self.n < 32 {
            self.refill();
        }
    }

    #[inline(always)]
    fn refill(&mut self) {
        if let Some(w) = self.data.get(self.next..).and_then(|s| s.first_chunk::<8>()) {
            // n < 32 here, so the shifts stay below 64
            self.buf |= u64::from_le_bytes(*w) << self.n;
            self.next += ((63 - self.n) >> 3) as usize;
            self.n |= 56;
        } else {
            self.refill_slow();
        }
    }

    #[cold]
    fn refill_slow(&mut self) {
        while self.n <= 56 {
            let byte = self.data.get(self.next).copied().unwrap_or(0);
            self.buf |= u64::from(byte) << self.n;
            self.next = self.next.saturating_add(1);
            self.n += 8;
        }
    }

    /// Drops `k <= 31` bits (callers keep at least 32 buffered with [`Bits::ensure`]).
    #[inline(always)]
    fn consume(&mut self, k: u32) {
        self.buf >>= k & 63;
        self.n = self.n.saturating_sub(k);
    }

    /// Reads `k <= 31` bits.
    #[inline(always)]
    fn take(&mut self, k: u32) -> usize {
        let v = (self.buf & ((1u64 << (k & 31)) - 1)) as usize;
        self.consume(k);
        v
    }

    fn consumed_bits(&self) -> u64 {
        (self.next as u64)
            .saturating_mul(8)
            .saturating_sub(u64::from(self.n))
    }
}

/// `HUFFMAN_DECODE_SYMBOL` on the 64-bit buffer. Needs 32 buffered bits.
#[inline(always)]
fn decode_symbol(
    bits: &mut Bits<'_>,
    table: &[DecodeEntry],
    root: usize,
    root_bits: u32,
    tail_bits: u32,
) -> Result<usize, Error> {
    let idx = root + (bits.buf & ((1u64 << root_bits) - 1)) as usize;
    let mut e = i32::from(*table.get(idx).ok_or(Error::CorruptedData("Huffman table walk"))?);
    if e < 0 {
        e = walk(bits, table, root, e, tail_bits)?;
    }
    bits.consume((e & 15) as u32);
    Ok((e >> 4) as usize)
}

/// The rare codewords longer than the root table: follow the links into the tail tables.
#[cold]
fn walk(
    bits: &mut Bits<'_>,
    table: &[DecodeEntry],
    root: usize,
    mut e: i32,
    tail_bits: u32,
) -> Result<i32, Error> {
    let mut next = root as isize;
    loop {
        bits.consume((e & 15) as u32);
        bits.ensure();
        next += (e & !15) as isize;
        let idx = next
            .checked_add((bits.buf & ((1u64 << tail_bits) - 1)) as isize)
            .and_then(|i| usize::try_from(i).ok())
            .ok_or(Error::CorruptedData("Huffman table walk"))?;
        e = i32::from(*table.get(idx).ok_or(Error::CorruptedData("Huffman table walk"))?);
        if e >= 0 {
            return Ok(e);
        }
    }
}

// ------------------------------------------------------------------------------------- copies

/// LZ77 copy inside `dst`: `len` bytes from `src` to `at` (`src < at`, overlap allowed).
/// With `overrun`, up to 15 bytes past `at + len` may be overwritten (positions not decoded
/// yet), which allows 16- and 8-byte steps.
#[inline(always)]
fn copy_local(dst: &mut [u8], src: usize, at: usize, len: usize, overrun: bool) -> Result<(), Error> {
    let bad = || Error::Internal("match copy range");
    let dist = at.checked_sub(src).filter(|d| *d > 0).ok_or_else(bad)?;
    let end = at.checked_add(len).filter(|e| *e <= dst.len()).ok_or_else(bad)?;
    if overrun && dist >= 16 && end + 16 <= dst.len() {
        // each step reads 16 bytes that end at or before the step's destination
        let mut i = 0;
        while i < len {
            let s = *dst
                .get(src + i..)
                .and_then(|s| s.first_chunk::<16>())
                .ok_or_else(bad)?;
            *dst.get_mut(at + i..)
                .and_then(|d| d.first_chunk_mut::<16>())
                .ok_or_else(bad)? = s;
            i += 16;
        }
        return Ok(());
    }
    if overrun && dist >= 8 && end + 8 <= dst.len() {
        let mut i = 0;
        while i < len {
            let s = *dst
                .get(src + i..)
                .and_then(|s| s.first_chunk::<8>())
                .ok_or_else(bad)?;
            *dst.get_mut(at + i..)
                .and_then(|d| d.first_chunk_mut::<8>())
                .ok_or_else(bad)? = s;
            i += 8;
        }
        return Ok(());
    }
    if dist >= len {
        // `src + len <= at` and `at + len <= dst.len()`: the ranges are valid
        dst.copy_within(src..src + len, at);
        return Ok(());
    }
    if dist == 1 {
        let b = *dst.get(src).ok_or_else(bad)?;
        dst.get_mut(at..end).ok_or_else(bad)?.fill(b);
        return Ok(());
    }
    if len <= 32 {
        for i in 0..len {
            let b = *dst.get(src + i).ok_or_else(bad)?;
            *dst.get_mut(at + i).ok_or_else(bad)? = b;
        }
        return Ok(());
    }
    // short period, long run: the copied prefix is periodic, so its length doubles each round
    let mut done = 0usize;
    while done < len {
        let n = (len - done).min(dist + done);
        dst.copy_within(src..src + n, at + done);
        done += n;
    }
    Ok(())
}

/// A match whose source starts `back` bytes before `at`, reading the history when `back > at`.
#[inline(always)]
fn copy_match(
    dst: &mut [u8],
    history: &History<'_, '_>,
    at: usize,
    back: usize,
    len: usize,
    overrun: bool,
) -> Result<(), Error> {
    if back <= at {
        return copy_local(dst, at - back, at, len, overrun);
    }
    let from_history = back - at;
    let n = len.min(from_history);
    let end = at.checked_add(n).ok_or(Error::Internal("match copy range"))?;
    history.copy_back(
        from_history,
        dst.get_mut(at..end).ok_or(Error::Internal("match copy range"))?,
    )?;
    if len > n {
        // the rest of the match reads the start of the job (`at + n - back == 0`)
        copy_local(dst, 0, end, len - n, overrun)?;
    }
    Ok(())
}

/// A match recorded by a deferred decode: `len` bytes to `at` (relative to the job) from
/// `back` bytes before it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pending {
    at: u32,
    back: u32,
    len: u32,
}

/// Replays the matches a deferred decode recorded, in order, once the history is final.
pub(crate) fn replay(pending: &[Pending], dst: &mut [u8], history: &History<'_, '_>) -> Result<(), Error> {
    for p in pending {
        copy_match(
            dst,
            history,
            p.at as usize,
            p.back as usize,
            p.len as usize,
            false,
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------- decoding

/// Entries of the literal table: one per value of the next 12 bits.
const LITERAL_TABLE_SIZE: usize = 1 << SHORT_ROOT_BITS;

/// Per-thread decoding state: Huffman tables and the deferred-mode bookkeeping.
pub(crate) struct Scratch {
    short: Vec<DecodeEntry>,
    long: Vec<DecodeEntry>,
    /// The literals the next 12 bits start with (see [`build_literal_table`]).
    literals: Box<[u32; LITERAL_TABLE_SIZE]>,
    /// Deferred mode: sorted, disjoint ranges of the job whose bytes are not known yet.
    unknown: Vec<(usize, usize)>,
}

impl Scratch {
    pub fn new() -> Scratch {
        Scratch {
            short: vec![0; SHORT_TABLE_SIZE],
            long: vec![0; LONG_TABLE_SIZE],
            literals: Box::new([0; LITERAL_TABLE_SIZE]),
            unknown: Vec::new(),
        }
    }
}

/// Literal-table entry fields: first literal (bits 0-7), second literal (8-15), length of the
/// first codeword (16-20), length of both (21-25), number of literals (30-31; 0 = none, decode
/// the general way).
const LIT_ONE: u32 = 1 << 30;
const LIT_TWO: u32 = 2 << 30;

/// For every value of the next 12 bits: the literal(s) they start with, when the first symbol
/// is a literal with a codeword of at most 12 bits, and the second one too if both codewords
/// fit in the 12 bits. Two literals per lookup halve the dependent table loads on text-like
/// data. Derived from the root of the short-symbol table, so it decodes exactly the same.
fn build_literal_table(short: &[DecodeEntry], root: usize, out: &mut [u32; LITERAL_TABLE_SIZE]) {
    let entry = |i: usize| -> Option<(u32, u32)> {
        let e = i32::from(*short.get(root + i)?);
        let (symbol, len) = (e >> 4, (e & 15) as u32);
        (e >= 0 && symbol < 256 && len >= 1).then_some((symbol as u32, len))
    };
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = match entry(i) {
            None => 0,
            Some((first, len1)) => {
                // the 12 - len1 bits after the first codeword pick the second root entry when
                // the second codeword is that short
                let rest = i >> len1;
                match entry(rest) {
                    Some((second, len2)) if len1 + len2 <= SHORT_ROOT_BITS => {
                        LIT_TWO | first | second << 8 | len1 << 16 | (len1 + len2) << 21
                    }
                    _ => LIT_ONE | first | len1 << 16 | len1 << 21,
                }
            }
        };
    }
}

/// How a job's history is available.
pub(crate) enum Source<'h, 's, 'a> {
    /// Known: matches into the history are copied.
    Known(&'h History<'s, 'a>),
    /// Unknown: matches that depend on it are recorded in `pending`, at most `cap` of them.
    Deferred {
        pending: &'h mut Vec<Pending>,
        cap: usize,
    },
}

/// Decodes the blocks of `job` into `dst[..job.out_len]` (`dst` may be longer: the extra room
/// lets match copies run over). Returns `false` if a deferred decode gave up because it would
/// record more than its cap of matches; `dst` is then incomplete.
pub(crate) fn decode_job(
    blocks: &[BlockRef<'_>],
    job: &Job,
    mut source: Source<'_, '_, '_>,
    dst: &mut [u8],
    scratch: &mut Scratch,
) -> Result<bool, Error> {
    if dst.len() < job.out_len {
        return Err(Error::Internal("job output too small"));
    }
    let empty = History::new(&[]);
    scratch.unknown.clear();
    let blocks = blocks
        .get(job.blocks.clone())
        .ok_or(Error::Internal("job blocks"))?;
    let mut at = 0usize;
    for block in blocks {
        let history = match &source {
            Source::Known(h) => Some(*h),
            Source::Deferred { .. } if job.history_len() == 0 => Some(&empty),
            Source::Deferred { .. } => None,
        };
        let pending = match &mut source {
            Source::Deferred { pending, cap } => Some((&mut **pending, *cap)),
            Source::Known(_) => None,
        };
        match decode_block(block, job, history, pending, at, dst, scratch)? {
            Some(end) => at = end,
            None => return Ok(false),
        }
    }
    if at != job.out_len {
        return Err(Error::Internal("job size"));
    }
    Ok(true)
}

/// `Xpress9DecodeState`: the MTF state and both Huffman tables of a block.
fn read_state(
    br: &mut BitReader<'_>,
    params: &SessionParams,
    scratch: &mut Scratch,
) -> Result<(bool, [usize; 4], usize, usize), Error> {
    let mut last_ptr = false;
    let mut offsets = [0usize; 4];
    if params.mtf_count > 0 {
        last_ptr = br.load(1) != 0;
        for i in 0..params.mtf_count {
            let msb = br.load(5);
            if msb >= params.window_size_log2 {
                return Err(Error::CorruptedData("MTF offset larger than the window"));
            }
            *offsets.get_mut(i).ok_or(Error::Internal("MTF index"))? =
                br.load(msb) as usize + (1usize << msb);
        }
    }
    let mut short_lengths = [0u8; LZ77_SHORT_SYMBOL_ALPHABET_SIZE];
    huffman_decode_length_table(
        br,
        &mut short_lengths,
        LZ77_MAX_SHORT_LENGTH,
        HUFFMAN_MAX_CODEWORD_LENGTH,
    )?;
    let short_root = huffman_create_decode_tables(
        &short_lengths,
        &mut scratch.short,
        LZ77_SHORT_SYMBOL_DECODE_ROOT_BITS,
        LZ77_SHORT_SYMBOL_DECODE_TAIL_BITS,
    )?;
    let mut long_lengths = [0u8; LZ77_LONG_LENGTH_ALPHABET_SIZE];
    huffman_decode_length_table(
        br,
        &mut long_lengths,
        LZ77_MAX_SHORT_LENGTH,
        HUFFMAN_MAX_CODEWORD_LENGTH,
    )?;
    let long_root = huffman_create_decode_tables(
        &long_lengths,
        &mut scratch.long,
        LZ77_LONG_LENGTH_DECODE_ROOT_BITS,
        LZ77_LONG_LENGTH_DECODE_TAIL_BITS,
    )?;
    Ok((last_ptr, offsets, short_root, long_root))
}

/// Is any byte of `start..start + len` (job positions) still unknown?
#[inline(always)]
fn is_unknown(unknown: &[(usize, usize)], start: usize, len: usize) -> bool {
    match unknown.last() {
        None => false,
        Some(&(_, last_end)) if last_end <= start => false,
        Some(_) => {
            let i = unknown.partition_point(|&(_, e)| e <= start);
            unknown.get(i).is_some_and(|&(s, _)| s < start + len)
        }
    }
}

/// Decodes one block at `at` within the job. `history` is `None` in deferred mode. Returns the
/// position after the block, or `None` if the deferred decode gave up.
#[allow(clippy::too_many_arguments)]
fn decode_block(
    block: &BlockRef<'_>,
    job: &Job,
    history: Option<&History<'_, '_>>,
    mut pending: Option<(&mut Vec<Pending>, usize)>,
    start: usize,
    dst: &mut [u8],
    scratch: &mut Scratch,
) -> Result<Option<usize>, Error> {
    let header = &block.header;
    let params = &job.params;
    let payload = block
        .bytes
        .get(BLOCK_HEADER_SIZE..)
        .ok_or(Error::Truncated("block payload"))?;
    let mut br = BitReader::new(payload);
    let (mut last_ptr, [mut off0, mut off1, mut off2, mut off3], short_root, long_root) =
        read_state(&mut br, params, scratch)?;
    if br.consumed_bits() != u64::from(header.huffman_table_bits()) {
        return Err(Error::CorruptedData("Huffman table size mismatch"));
    }
    let payload_bits = u64::from(header.encoded_size_bits) - 8 * BLOCK_HEADER_SIZE as u64;
    let mut bits = Bits::new(payload, br.consumed_bits());

    let end = start
        .checked_add(header.original_size as usize)
        .filter(|e| *e <= job.out_len)
        .ok_or(Error::CorruptedData("block size mismatch"))?;
    let window = 1usize << params.window_size_log2;
    let hist_len = job.history_len();
    let mtf_count = params.mtf_count;
    let Scratch {
        short,
        long,
        literals,
        unknown,
    } = scratch;
    build_literal_table(short, short_root, literals);
    let (short, long, literals) = (&short[..], &long[..], &**literals);
    let empty = History::new(&[]);

    let mut p = start;
    while p < end {
        bits.ensure();
        // literals first, two at a time when their codewords are short
        let lit = literals
            .get((bits.buf & (LITERAL_TABLE_SIZE as u64 - 1)) as usize)
            .copied()
            .unwrap_or(0);
        if lit >= LIT_TWO && p + 2 <= end {
            bits.consume((lit >> 21) & 31);
            *dst.get_mut(p..)
                .and_then(|d| d.first_chunk_mut::<2>())
                .ok_or(Error::Internal("literal position"))? = [lit as u8, (lit >> 8) as u8];
            p += 2;
            last_ptr = false;
            continue;
        }
        if lit >= LIT_ONE {
            bits.consume((lit >> 16) & 31);
            *dst.get_mut(p).ok_or(Error::Internal("literal position"))? = lit as u8;
            p += 1;
            last_ptr = false;
            continue;
        }
        let mut symbol = decode_symbol(&mut bits, short, short_root, SHORT_ROOT_BITS, SHORT_TAIL_BITS)?;
        if symbol < 256 {
            *dst.get_mut(p).ok_or(Error::Internal("literal position"))? = symbol as u8;
            p += 1;
            last_ptr = false;
            continue;
        }
        symbol -= 256;

        // a pointer: the length first
        let mut length = symbol & (LZ77_MAX_SHORT_LENGTH - 1);
        symbol >>= LZ77_MAX_SHORT_LENGTH_LOG;
        if length == LZ77_MAX_SHORT_LENGTH - 1 {
            bits.ensure();
            length = decode_symbol(&mut bits, long, long_root, LONG_ROOT_BITS, LONG_TAIL_BITS)?;
            if length >= LZ77_MAX_LONG_LENGTH {
                let extra = (length - LZ77_MAX_LONG_LENGTH) as u32;
                if extra > 31 {
                    return Err(Error::CorruptedData("long length width"));
                }
                bits.ensure();
                length = bits.take(extra) + (1usize << extra) + LZ77_MAX_LONG_LENGTH - 1;
            }
            length += LZ77_MAX_SHORT_LENGTH - 1;
        }

        let offset: usize;
        if mtf_count > 0 && symbol < mtf_count {
            // an MTF pointer
            length += params.mtf_min;
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
            // a regular pointer: the offset follows
            length += params.ptr_min;
            let width = (symbol - mtf_count) as u32;
            if width > 31 {
                return Err(Error::CorruptedData("offset width"));
            }
            bits.ensure();
            offset = bits.take(width) + (1usize << width);
            if mtf_count > 2 {
                off3 = off2;
                off2 = off1;
            }
            if mtf_count > 0 {
                off1 = off0;
                off0 = offset;
            }
        }
        last_ptr = true;

        // the stricter reach (see the module notes), then the copy
        if offset == 0 || offset > window || offset > hist_len.saturating_add(p) {
            return Err(Error::CorruptedData("match offset before start of data"));
        }
        let match_end = p
            .checked_add(length)
            .filter(|e| *e <= end)
            .ok_or(Error::CorruptedData("match runs past the block"))?;
        let known = match history {
            Some(_) => true,
            // deferred: known when the source lies in the job and none of it is unknown yet
            None => offset <= p && !is_unknown(unknown, p - offset, length.min(offset)),
        };
        if known {
            copy_match(dst, history.unwrap_or(&empty), p, offset, length, true)?;
        } else if let Some((list, cap)) = pending.as_mut() {
            if list.len() >= *cap {
                return Ok(None);
            }
            let to_u32 = |v: usize| u32::try_from(v).map_err(|_| Error::Internal("job too large"));
            list.push(Pending {
                at: to_u32(p)?,
                back: to_u32(offset)?,
                len: to_u32(length)?,
            });
            match unknown.last_mut() {
                Some(last) if last.1 == p => last.1 = match_end,
                _ => unknown.push((p, match_end)),
            }
        } else {
            return Err(Error::Internal("deferred decode without a pending list"));
        }
        p = match_end;
    }
    if bits.consumed_bits() != payload_bits {
        return Err(Error::CorruptedData("block size mismatch"));
    }
    Ok(Some(end))
}
