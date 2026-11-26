//! The framing of the PBIX `DataModel` part (and of PBIP `.pbi/cache.abf`) around XPress9.
//!
//! This module is original bicraft code (not part of the Microsoft port). The layout follows
//! the CC0-1.0 Kaitai description `abf_xpress9.ksy` from Hugoberry/pbixray
//! (<https://github.com/Hugoberry/pbixray/blob/main/docs/abf_xpress9.ksy>) and
//! `docs/formats/datamodel-abf.md` / `docs/formats/xpress9.md`:
//!
//! ```text
//! banner      102 bytes, UTF-16LE text; "multithreaded" at character 30 marks the
//!             multithreaded layout
//! single:     chunk*  until end of part
//! multi:      u64 main_chunks_per_thread, u64 prefix_chunks_per_thread,
//!             u64 prefix_thread_count, u64 main_thread_count, u64 chunk_uncompressed_size
//!             prefix chunks (prefix_thread_count groups of prefix_chunks_per_thread)
//!             main chunks   (main_thread_count groups of main_chunks_per_thread)
//! chunk:      u32 uncompressed size, u32 compressed size, XPress9 block(s)
//! ```
//!
//! A single-threaded part is one XPress9 session: history carries across chunks. In the
//! multithreaded layout each thread's group of chunks is its own session. The output is the
//! concatenation of all chunks in stored order.
//!
//! A part that is already a bare ABF (it starts with the [MS-XLDM] stream storage signature)
//! is returned unchanged.
//!
//! [`compress_datamodel`] writes the single-threaded layout (see `docs/formats/xpress9.md`,
//! "Writing the DataModel framing"), either with one encoder over the whole part
//! ([`CompressionMode::Exact`]) or with independently primed chunk encoders running in
//! parallel ([`CompressionMode::Balanced`], [`CompressionMode::Fast`], the default).

use crate::dec_fast::Unit;
use crate::dec_lz77::{BlockHeader, Decoder};
use crate::dec_par;
use crate::enc_lz77::{Encoder, EncoderParams};
use crate::misc::xpress9_crc32;
use crate::{Error, WINDOW_SIZE_LOG2_MAX, WINDOW_SIZE_LOG2_MIN};

/// Size of the UTF-16LE banner at the start of an XPress9-framed part.
pub const BANNER_SIZE: usize = 102;
/// Size of the multithreaded thread-distribution header that follows the banner.
pub const MULTITHREADED_HEADER_SIZE: usize = 40;
/// Size of a chunk envelope (uncompressed size, compressed size).
pub const CHUNK_HEADER_SIZE: usize = 8;

/// The UTF-16LE marker that tells the multithreaded banner apart (characters 30..43).
const MULTITHREADED_MARKER: &str = "multithreaded";
const MULTITHREADED_MARKER_CHAR_OFFSET: usize = 30;

/// Interoperability identifier: the text of the single-threaded banner, which Power BI Desktop
/// (2.158.1304.0) writes at the start of every sequentially compressed `DataModel`. Like a
/// file signature it is a format magic, not content: readers tell the layouts apart by it.
/// Stored as UTF-16LE followed by one UTF-16 NUL, which fills exactly [`BANNER_SIZE`] bytes.
pub const SINGLE_THREADED_BANNER_TEXT: &str = "This backup was created using XPress9 compression.";

/// Uncompressed size of each chunk the writer emits when no sample says otherwise: 2 MiB, the
/// size observed in Desktop's chunk envelopes and multithreaded headers
/// (`docs/formats/datamodel-abf.md`).
pub const DEFAULT_CHUNK_SIZE: usize = 2 << 20;

/// Largest chunk the engine loads from a single-threaded part: Desktop 2.158's engine refuses
/// a chunk envelope of more than 2 MiB (2 MiB + 1 byte already fails, G17,
/// `docs/formats/xpress9.md` §6), so [`compress_with_plan`] refuses larger chunk sizes.
pub const MAX_CHUNK_SIZE: usize = 2 << 20;

/// Chunk sizes taken from a sample are accepted in this range; anything else falls back to
/// [`DEFAULT_CHUNK_SIZE`].
const CHUNK_SIZE_RANGE: std::ops::RangeInclusive<u64> = (64 << 10)..=(MAX_CHUNK_SIZE as u64);

/// [MS-XLDM] §2.1.1: an ABF stream starts with a UTF-16LE byte-order mark and this signature.
const ABF_SIGNATURE_PREFIX: &str = "STREAM_STORAGE_SIGNATURE";

/// How a `DataModel` part is framed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Not XPress9-compressed: the part is an ABF already.
    Uncompressed,
    /// Banner, then a flat sequence of chunks forming one XPress9 session.
    SingleThreaded,
    /// Banner, thread-distribution header, then per-thread chunk groups.
    MultiThreaded,
}

/// The 40-byte thread-distribution header of the multithreaded layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultiThreadedHeader {
    pub main_chunks_per_thread: u64,
    pub prefix_chunks_per_thread: u64,
    pub prefix_thread_count: u64,
    pub main_thread_count: u64,
    pub chunk_uncompressed_size: u64,
}

/// One chunk envelope and the XPress9 blocks it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkInfo {
    /// Offset of the chunk envelope within the part.
    pub offset: usize,
    /// Index of the XPress9 session (0 for single-threaded parts; one per thread group otherwise).
    pub session: usize,
    pub uncompressed_size: u32,
    pub compressed_size: u32,
    /// Headers of the blocks inside the chunk.
    pub blocks: Vec<BlockHeader>,
}

/// Framing summary of a `DataModel` part (no decompression involved).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataModelInfo {
    pub layout: Layout,
    /// The [`BANNER_SIZE`] banner bytes as stored (empty for [`Layout::Uncompressed`]).
    pub banner: Vec<u8>,
    pub multithreaded_header: Option<MultiThreadedHeader>,
    pub chunks: Vec<ChunkInfo>,
    /// Number of XPress9 sessions.
    pub sessions: usize,
    /// Sum of the chunks' uncompressed sizes.
    pub total_uncompressed: u64,
    /// Bytes after the last chunk of a multithreaded part (not decoded).
    pub trailing_bytes: usize,
}

fn utf16_units(bytes: &[u8]) -> impl Iterator<Item = u16> + '_ {
    bytes.as_chunks::<2>().0.iter().map(|p| u16::from_le_bytes(*p))
}

fn utf16_matches(bytes: &[u8], char_offset: usize, text: &str) -> bool {
    let Some(start) = char_offset.checked_mul(2) else {
        return false;
    };
    let Some(window) = bytes.get(start..) else {
        return false;
    };
    let mut units = utf16_units(window);
    text.encode_utf16().all(|t| units.next() == Some(t))
}

/// Recognise the framing of a `DataModel` part from its first bytes.
pub fn detect_layout(part: &[u8]) -> Result<Layout, Error> {
    if part.get(..2) == Some(&[0xff, 0xfe])
        && utf16_matches(part.get(2..).unwrap_or(&[]), 0, ABF_SIGNATURE_PREFIX)
    {
        return Ok(Layout::Uncompressed);
    }
    let banner = part
        .get(..BANNER_SIZE)
        .ok_or(Error::Framing("part shorter than the XPress9 banner"))?;
    if utf16_matches(banner, MULTITHREADED_MARKER_CHAR_OFFSET, MULTITHREADED_MARKER) {
        Ok(Layout::MultiThreaded)
    } else {
        Ok(Layout::SingleThreaded)
    }
}

fn read_u32_at(part: &[u8], offset: usize) -> Result<u32, Error> {
    part.get(offset..)
        .and_then(|s| s.first_chunk::<4>())
        .map(|w| u32::from_le_bytes(*w))
        .ok_or(Error::Framing("truncated chunk header"))
}

fn read_u64_at(part: &[u8], offset: usize) -> Result<u64, Error> {
    part.get(offset..)
        .and_then(|s| s.first_chunk::<8>())
        .map(|w| u64::from_le_bytes(*w))
        .ok_or(Error::Framing("truncated multithreaded header"))
}

/// A chunk located in the part: (envelope offset, uncompressed size, payload).
struct RawChunk<'a> {
    offset: usize,
    session: usize,
    uncompressed_size: u32,
    payload: &'a [u8],
}

fn read_chunk(part: &[u8], offset: usize, session: usize) -> Result<(RawChunk<'_>, usize), Error> {
    let uncompressed_size = read_u32_at(part, offset)?;
    let compressed_size = read_u32_at(part, offset + 4)?;
    let start = offset + CHUNK_HEADER_SIZE;
    let end = start
        .checked_add(compressed_size as usize)
        .ok_or(Error::Framing("chunk size overflow"))?;
    let payload = part
        .get(start..end)
        .ok_or(Error::Framing("chunk extends past the end of the part"))?;
    Ok((
        RawChunk {
            offset,
            session,
            uncompressed_size,
            payload,
        },
        end,
    ))
}

struct Parsed<'a> {
    layout: Layout,
    header: Option<MultiThreadedHeader>,
    chunks: Vec<RawChunk<'a>>,
    sessions: usize,
    trailing_bytes: usize,
}

fn parse(part: &[u8]) -> Result<Parsed<'_>, Error> {
    let layout = detect_layout(part)?;
    let mut chunks = Vec::new();
    match layout {
        Layout::Uncompressed => Ok(Parsed {
            layout,
            header: None,
            chunks,
            sessions: 0,
            trailing_bytes: 0,
        }),
        Layout::SingleThreaded => {
            let mut offset = BANNER_SIZE;
            while offset < part.len() {
                let (chunk, end) = read_chunk(part, offset, 0)?;
                chunks.push(chunk);
                offset = end;
            }
            Ok(Parsed {
                layout,
                header: None,
                chunks,
                sessions: 1,
                trailing_bytes: 0,
            })
        }
        Layout::MultiThreaded => {
            let base = BANNER_SIZE;
            let header = MultiThreadedHeader {
                main_chunks_per_thread: read_u64_at(part, base)?,
                prefix_chunks_per_thread: read_u64_at(part, base + 8)?,
                prefix_thread_count: read_u64_at(part, base + 16)?,
                main_thread_count: read_u64_at(part, base + 24)?,
                chunk_uncompressed_size: read_u64_at(part, base + 32)?,
            };
            let mut offset = base + MULTITHREADED_HEADER_SIZE;
            // every chunk takes at least 8 bytes, which bounds the counts before any loop runs
            let max_chunks = (part.len() / CHUNK_HEADER_SIZE) as u64;
            let groups = [
                (header.prefix_thread_count, header.prefix_chunks_per_thread),
                (header.main_thread_count, header.main_chunks_per_thread),
            ];
            let mut total: u64 = 0;
            for (threads, per_thread) in groups {
                if threads == 0 || per_thread == 0 {
                    continue;
                }
                let n = threads
                    .checked_mul(per_thread)
                    .ok_or(Error::Framing("chunk count overflow"))?;
                total = total
                    .checked_add(n)
                    .ok_or(Error::Framing("chunk count overflow"))?;
                if total > max_chunks {
                    return Err(Error::Framing("more chunks announced than the part can hold"));
                }
            }
            let mut session = 0usize;
            for (threads, per_thread) in groups {
                if threads == 0 || per_thread == 0 {
                    continue;
                }
                for _ in 0..threads {
                    for _ in 0..per_thread {
                        let (chunk, end) = read_chunk(part, offset, session)?;
                        chunks.push(chunk);
                        offset = end;
                    }
                    session += 1;
                }
            }
            Ok(Parsed {
                layout,
                header: Some(header),
                chunks,
                sessions: session,
                trailing_bytes: part.len() - offset,
            })
        }
    }
}

/// Describe the framing of a `DataModel` part: layout, chunk envelopes and block headers.
pub fn inspect_datamodel(part: &[u8]) -> Result<DataModelInfo, Error> {
    let parsed = parse(part)?;
    let mut chunks = Vec::with_capacity(parsed.chunks.len());
    let mut total_uncompressed: u64 = 0;
    for c in &parsed.chunks {
        let mut blocks = Vec::new();
        let mut at = 0usize;
        while at < c.payload.len() {
            let h = BlockHeader::parse(c.payload.get(at..).unwrap_or(&[]))?;
            at = at
                .checked_add(h.block_size())
                .ok_or(Error::Framing("block size overflow"))?;
            if at > c.payload.len() {
                return Err(Error::Truncated("block extends past its chunk"));
            }
            blocks.push(h);
        }
        total_uncompressed += u64::from(c.uncompressed_size);
        chunks.push(ChunkInfo {
            offset: c.offset,
            session: c.session,
            uncompressed_size: c.uncompressed_size,
            compressed_size: c.payload.len() as u32,
            blocks,
        });
    }
    if parsed.layout == Layout::Uncompressed {
        total_uncompressed = part.len() as u64;
    }
    let banner = match parsed.layout {
        Layout::Uncompressed => Vec::new(),
        _ => part.get(..BANNER_SIZE).unwrap_or_default().to_vec(),
    };
    Ok(DataModelInfo {
        layout: parsed.layout,
        banner,
        multithreaded_header: parsed.header,
        chunks,
        sessions: parsed.sessions,
        total_uncompressed,
        trailing_bytes: parsed.trailing_bytes,
    })
}

/// Decompress a PBIX `DataModel` part (or a PBIP `cache.abf`) to the raw ABF bytes.
///
/// `limit` caps the decompressed size; exceeding it is an error, as is any chunk whose
/// decompressed size differs from its envelope. The sizes the chunk envelopes declare are
/// summed and checked against `limit` before anything is decoded or allocated.
///
/// Decodes on up to [`MAX_DECODE_THREADS`](crate::MAX_DECODE_THREADS) threads
/// (`docs/formats/xpress9.md` §7); the output is allocated in one piece once every block header
/// has been checked. The result, and the error for a malformed part, are the same as
/// [`decompress_datamodel_reference`]'s.
pub fn decompress_datamodel(part: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    decompress_datamodel_with(part, limit, 0)
}

/// [`decompress_datamodel`] on at most `threads` threads (0 = automatic, at most
/// [`MAX_DECODE_THREADS`](crate::MAX_DECODE_THREADS); 1 = the calling thread only). The output
/// does not depend on it.
pub fn decompress_datamodel_with(part: &[u8], limit: usize, threads: usize) -> Result<Vec<u8>, Error> {
    let parsed = parse(part)?;
    if let Some(early) = check_declared(part, &parsed, limit) {
        return early;
    }
    if let Some(out) = dec_par::decode(&units(&parsed), WINDOW_SIZE_LOG2_MAX, threads) {
        return Ok(out);
    }
    // a malformed (or merely unusual) stream: the reference decoder has the last word
    decompress_parsed_reference(&parsed, limit)
}

/// The reference decoder: one [`Decoder`] on the calling thread, a direct port of upstream's
/// streaming decoder. [`decompress_datamodel`] gives the same bytes and errors, faster.
pub fn decompress_datamodel_reference(part: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    let parsed = parse(part)?;
    if let Some(early) = check_declared(part, &parsed, limit) {
        return early;
    }
    decompress_parsed_reference(&parsed, limit)
}

/// Checks that `part` decompresses to exactly `expected` (`Ok(false)` if it decodes to other
/// bytes; an error if it does not decode within `expected.len()` bytes). This is what
/// `decompress_datamodel(part, expected.len())? == expected` answers, but without building the
/// output: the blocks are decoded in parallel, each against `expected` as its history, and
/// compared as they come (`docs/formats/xpress9.md` §7).
pub fn verify_datamodel(part: &[u8], expected: &[u8]) -> Result<bool, Error> {
    verify_datamodel_with(part, expected, 0)
}

/// [`verify_datamodel`] on at most `threads` threads (0 = automatic).
pub fn verify_datamodel_with(part: &[u8], expected: &[u8], threads: usize) -> Result<bool, Error> {
    let parsed = parse(part)?;
    if parsed.layout == Layout::Uncompressed {
        return Ok(part == expected);
    }
    if dec_par::verify(&units(&parsed), expected, WINDOW_SIZE_LOG2_MAX, threads) == Some(true) {
        return Ok(true);
    }
    // not verified: let the reference decoder give the exact answer (a mismatch or an error)
    Ok(decompress_datamodel_reference(part, expected.len())? == expected)
}

/// Whether the fast decoder alone decodes `part` (on the calling thread), without falling back
/// to the reference decoder. A diagnostic for tests and benchmarks.
#[doc(hidden)]
pub fn decodes_on_fast_path(part: &[u8]) -> bool {
    parse(part).is_ok_and(|parsed| {
        parsed.layout != Layout::Uncompressed
            && dec_par::decode(&units(&parsed), WINDOW_SIZE_LOG2_MAX, 1).is_some()
    })
}

/// The chunks as decoder units: (session, payload, expected size).
fn units<'a>(parsed: &Parsed<'a>) -> Vec<Unit<'a>> {
    parsed
        .chunks
        .iter()
        .map(|c| (c.session, c.payload, c.uncompressed_size as usize))
        .collect()
}

/// The checks done before decoding: an uncompressed part is returned as is, and declared
/// sizes beyond `limit` are refused. `None` when decoding should go ahead.
fn check_declared(part: &[u8], parsed: &Parsed<'_>, limit: usize) -> Option<Result<Vec<u8>, Error>> {
    if parsed.layout == Layout::Uncompressed {
        if part.len() > limit {
            return Some(Err(Error::LimitExceeded(limit)));
        }
        return Some(Ok(part.to_vec()));
    }
    let declared = parsed
        .chunks
        .iter()
        .try_fold(0usize, |acc, c| acc.checked_add(c.uncompressed_size as usize));
    if declared.is_none_or(|d| d > limit) {
        return Some(Err(Error::LimitExceeded(limit)));
    }
    None
}

fn decompress_parsed_reference(parsed: &Parsed<'_>, limit: usize) -> Result<Vec<u8>, Error> {
    let mut decoder = Decoder::new(WINDOW_SIZE_LOG2_MAX)?;
    let mut out: Vec<u8> = Vec::new();
    let mut current_session = usize::MAX;
    for chunk in &parsed.chunks {
        if chunk.session != current_session {
            decoder.start_session();
            current_session = chunk.session;
        }
        let expected = chunk.uncompressed_size as usize;
        let room = limit.saturating_sub(out.len());
        if expected > room {
            return Err(Error::LimitExceeded(limit));
        }
        out.try_reserve(expected.min(64 << 20))
            .map_err(|_| Error::LimitExceeded(limit))?;
        let produced = decoder.decode_blocks(chunk.payload, &mut out, expected)?;
        if produced != expected {
            return Err(Error::SizeMismatch {
                expected,
                actual: produced,
            });
        }
    }
    Ok(out)
}

/// The 102-byte single-threaded banner: [`SINGLE_THREADED_BANNER_TEXT`] as UTF-16LE, then NUL.
pub fn single_threaded_banner() -> Vec<u8> {
    let mut out: Vec<u8> = SINGLE_THREADED_BANNER_TEXT
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    out.resize(BANNER_SIZE, 0);
    out
}

/// How [`compress_with_plan`] runs the encoder. Every mode writes the same single-threaded
/// layout (one XPress9 session: one session signature, block indices counting across chunks)
/// with the plan's window, MTF and minimum-match settings, and every mode is deterministic: the
/// same input and plan give the same bytes on every platform and for every thread count.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CompressionMode {
    /// One encoder runs over the whole part, so history and MTF state carry across chunks.
    /// With [`EncoderParams::power_bi_desktop`] this reproduces the stream Desktop writes,
    /// byte for byte apart from the session signature and the header CRC. Single-threaded,
    /// and the slowest mode.
    Exact,
    /// Each chunk is compressed by its own encoder, primed with up to [`PARALLEL_HISTORY`]
    /// bytes of the input that precede the chunk (so matches still reach back into earlier
    /// chunks) and started with a fresh MTF state. The chunks run in parallel and their blocks
    /// are then renumbered into one session. Any XPress9 decoder reads the result as one
    /// session (each block header carries its own MTF state), but it is not Desktop's byte
    /// sequence. Same match search as the plan, so about as small as [`CompressionMode::Exact`].
    Balanced,
    /// [`CompressionMode::Balanced`] with a lookup depth of at most 1, which also selects
    /// upstream's small 2^12-entry hash table: several times faster again per thread, for a
    /// part a few percent larger. The default.
    #[default]
    Fast,
}

/// [`CompressionMode::Balanced`] and [`CompressionMode::Fast`]: bytes of preceding input each
/// chunk's encoder is primed with (capped at half the window).
pub const PARALLEL_HISTORY: usize = 256 << 10;

/// [`FramingPlan::threads`] = 0 uses the machine's parallelism, capped at this many threads
/// (each worker holds an encoder of about `9 * 2^window` bytes).
pub const MAX_AUTO_THREADS: usize = 8;

/// The writer's settings, derived from the framing of the part being replaced (or defaults).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FramingPlan {
    /// [`BANNER_SIZE`] bytes.
    pub banner: Vec<u8>,
    pub params: EncoderParams,
    /// `Xpress9Flag_*` runtime flags of the encoder.
    pub encoder_flags: u32,
    /// Uncompressed bytes per chunk.
    pub chunk_size: usize,
    /// Exact (Desktop's bytes, sequential), balanced or fast (parallel chunks; fast is the
    /// default).
    pub mode: CompressionMode,
    /// Worker threads of the parallel modes; 0 = automatic (see [`MAX_AUTO_THREADS`]). The
    /// output does not depend on it.
    pub threads: usize,
}

impl FramingPlan {
    /// The settings [`compress_datamodel`] uses for `like`:
    ///
    /// - banner: the sample's own bytes when the sample is single-threaded (Desktop's banner,
    ///   byte for byte), else [`single_threaded_banner`];
    /// - encoder: [`EncoderParams::power_bi_desktop`], with the window size, MTF entry count and
    ///   minimum match lengths of the sample's first block header (the lookup depth and the
    ///   lazy evaluation are not recorded in a stream);
    /// - chunk size: the first chunk's size of a single-threaded sample with several chunks, the
    ///   `chunk_uncompressed_size` of a multithreaded sample, else [`DEFAULT_CHUNK_SIZE`];
    /// - mode: [`CompressionMode::Fast`], with an automatic thread count.
    pub fn new(like: Option<&DataModelInfo>) -> FramingPlan {
        let (mut params, encoder_flags) = EncoderParams::power_bi_desktop();
        let mut banner = single_threaded_banner();
        let mut chunk_size = DEFAULT_CHUNK_SIZE;
        if let Some(info) = like {
            if info.layout == Layout::SingleThreaded
                && info.banner.len() == BANNER_SIZE
                && detect_layout(&info.banner).ok() == Some(Layout::SingleThreaded)
            {
                banner.clone_from(&info.banner);
            }
            if let Some(b) = info.chunks.first().and_then(|c| c.blocks.first()) {
                let window = b.window_size_log2();
                if (WINDOW_SIZE_LOG2_MIN..=WINDOW_SIZE_LOG2_MAX).contains(&window) {
                    params.window_size_log2 = window;
                }
                if matches!(b.mtf_entry_count(), 0 | 2 | 4) {
                    params.mtf_entry_count = b.mtf_entry_count();
                }
                let (ptr, mtf) = (b.ptr_min_match_length(), b.mtf_min_match_length());
                if (3..=4).contains(&ptr) && (2..=3).contains(&mtf) {
                    params.ptr_min_match_length = ptr;
                    params.mtf_min_match_length = mtf;
                }
            }
            let sampled = match (info.layout, info.multithreaded_header) {
                (Layout::SingleThreaded, _) if info.chunks.len() > 1 => {
                    info.chunks.first().map(|c| u64::from(c.uncompressed_size))
                }
                (Layout::MultiThreaded, Some(h)) => Some(h.chunk_uncompressed_size),
                _ => None,
            };
            if let Some(size) = sampled.filter(|s| CHUNK_SIZE_RANGE.contains(s)) {
                chunk_size = usize::try_from(size).unwrap_or(DEFAULT_CHUNK_SIZE);
            }
        }
        FramingPlan {
            banner,
            params,
            encoder_flags,
            chunk_size,
            mode: CompressionMode::default(),
            threads: 0,
        }
    }

    /// The same plan with another [`CompressionMode`].
    pub fn with_mode(mut self, mode: CompressionMode) -> FramingPlan {
        self.mode = mode;
        self
    }
}

/// A session seed derived from the content (FNV-1a over the length and the first and last
/// 64 KiB, mixed with their CRC), so the output is reproducible for a given input. The decoder
/// only checks that the signature is constant within a session (docs/formats/xpress9.md §6.2).
fn session_seed(abf: &[u8]) -> u64 {
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let head = abf.get(..abf.len().min(1 << 16)).unwrap_or_default();
    let tail = abf.get(abf.len().saturating_sub(1 << 16)..).unwrap_or_default();
    for b in (abf.len() as u64).to_le_bytes().iter().chain(head).chain(tail) {
        h ^= u64::from(*b);
        h = h.wrapping_mul(PRIME);
    }
    h ^ u64::from(xpress9_crc32(tail, xpress9_crc32(head, 0)))
}

/// Compresses an ABF into a PBIX `DataModel` part (or a PBIP `cache.abf`) in the
/// single-threaded layout Desktop writes: the 102-byte banner, then for every
/// [`FramingPlan::chunk_size`] bytes of input one chunk envelope (u32 uncompressed size, u32
/// compressed size) holding the XPress9 blocks of that chunk. All chunks belong to one
/// decoder session (one session signature, block indices counting across chunks); each chunk
/// is flushed, so it decodes to exactly its envelope size.
///
/// With `like` (the [`inspect_datamodel`] of the part being replaced) the banner, encoder
/// parameters and chunk size follow that part (see [`FramingPlan::new`]). The chunks are
/// compressed in parallel with a shallow match search ([`CompressionMode::Fast`]); through
/// [`compress_with_plan`] with [`CompressionMode::Exact`], re-compressing the ABF a Desktop
/// part decodes to gives Desktop's bytes back apart from each block's session signature and
/// header CRC.
///
/// Round trip: `decompress_datamodel(&compress_datamodel(abf, like)?, abf.len()) == abf`.
pub fn compress_datamodel(abf: &[u8], like: Option<&DataModelInfo>) -> Result<Vec<u8>, Error> {
    compress_with_plan(abf, &FramingPlan::new(like))
}

/// [`compress_datamodel`] with explicit settings.
pub fn compress_with_plan(abf: &[u8], plan: &FramingPlan) -> Result<Vec<u8>, Error> {
    if plan.banner.len() != BANNER_SIZE {
        return Err(Error::BadArgument("the banner must be 102 bytes"));
    }
    if plan.chunk_size == 0 || plan.chunk_size > MAX_CHUNK_SIZE {
        return Err(Error::BadArgument("chunk size out of range (1 byte to 2 MiB)"));
    }
    let mut out = Vec::new();
    out.try_reserve(BANNER_SIZE + abf.len() / 4)
        .map_err(|_| Error::LimitExceeded(abf.len()))?;
    out.extend_from_slice(&plan.banner);
    if abf.is_empty() {
        return Ok(out);
    }
    let seed = session_seed(abf);
    match plan.mode {
        CompressionMode::Exact => {
            let mut encoder = Encoder::with_flags(plan.params.window_size_log2, plan.encoder_flags)?;
            encoder.start_session(&plan.params, seed)?;
            for chunk in abf.chunks(plan.chunk_size) {
                let payload = encoder.compress(chunk, true)?;
                push_chunk(&mut out, chunk.len(), &payload)?;
            }
        }
        CompressionMode::Balanced | CompressionMode::Fast => {
            let mut plan = plan.clone();
            if plan.mode == CompressionMode::Fast {
                plan.params.lookup_depth = plan.params.lookup_depth.min(1);
            }
            let payloads = compress_chunks_parallel(abf, &plan, seed)?;
            let mut block_index: u32 = 0;
            for (chunk, mut payload) in abf.chunks(plan.chunk_size).zip(payloads) {
                renumber_blocks(&mut payload, &mut block_index)?;
                push_chunk(&mut out, chunk.len(), &payload)?;
            }
        }
    }
    Ok(out)
}

/// Appends one chunk envelope and its payload.
fn push_chunk(out: &mut Vec<u8>, uncompressed: usize, payload: &[u8]) -> Result<(), Error> {
    let uncompressed = u32::try_from(uncompressed).map_err(|_| Error::Internal("chunk size"))?;
    let compressed =
        u32::try_from(payload.len()).map_err(|_| Error::Framing("a compressed chunk exceeds 4 GiB"))?;
    out.try_reserve(CHUNK_HEADER_SIZE + payload.len())
        .map_err(|_| Error::LimitExceeded(payload.len()))?;
    out.extend_from_slice(&uncompressed.to_le_bytes());
    out.extend_from_slice(&compressed.to_le_bytes());
    out.extend_from_slice(payload);
    Ok(())
}

/// Rewrites the block index (header word 6) of every block in `payload` so they continue the
/// session's count from `*next`, and recomputes each header CRC (word 7).
fn renumber_blocks(payload: &mut [u8], next: &mut u32) -> Result<(), Error> {
    let mut at = 0usize;
    while at < payload.len() {
        let block = payload.get_mut(at..).ok_or(Error::Internal("block range"))?;
        let size = BlockHeader::parse(block)?.block_size();
        let header = block
            .get_mut(..crate::BLOCK_HEADER_SIZE)
            .ok_or(Error::Internal("block header"))?;
        header
            .get_mut(24..28)
            .ok_or(Error::Internal("block header"))?
            .copy_from_slice(&next.to_le_bytes());
        let crc = xpress9_crc32(header.get(..28).ok_or(Error::Internal("block header"))?, 0);
        header
            .get_mut(28..32)
            .ok_or(Error::Internal("block header"))?
            .copy_from_slice(&crc.to_le_bytes());
        *next = next
            .checked_add(1)
            .ok_or(Error::Framing("too many XPress9 blocks"))?;
        at = at
            .checked_add(size)
            .filter(|&a| a <= payload.len() && size > 0)
            .ok_or(Error::Internal("block size"))?;
    }
    Ok(())
}

/// The worker count for `chunks` chunks.
fn worker_count(plan: &FramingPlan, chunks: usize) -> usize {
    let wanted = if plan.threads == 0 {
        std::thread::available_parallelism()
            .map_or(1, std::num::NonZero::get)
            .min(MAX_AUTO_THREADS)
    } else {
        plan.threads
    };
    wanted.clamp(1, chunks.max(1))
}

/// Compresses one chunk for the parallel modes: a fresh session (block indices from 0),
/// primed with the input that precedes the chunk.
fn compress_one_chunk(
    encoder: &mut Encoder,
    abf: &[u8],
    index: usize,
    plan: &FramingPlan,
    seed: u64,
) -> Result<Vec<u8>, Error> {
    let start = index
        .checked_mul(plan.chunk_size)
        .ok_or(Error::Internal("chunk offset"))?;
    let end = start.saturating_add(plan.chunk_size).min(abf.len());
    let chunk = abf.get(start..end).ok_or(Error::Internal("chunk range"))?;
    let history = abf
        .get(start.saturating_sub(PARALLEL_HISTORY)..start)
        .ok_or(Error::Internal("history range"))?;
    encoder.start_session(&plan.params, seed)?;
    if !history.is_empty() {
        encoder.prime(history)?;
    }
    encoder.compress(chunk, true)
}

/// The payload of every chunk, in order, compressed independently on up to
/// [`worker_count`] threads. Workers take chunk indices from a shared counter; each result is
/// stored at its index, so the output does not depend on scheduling.
fn compress_chunks_parallel(abf: &[u8], plan: &FramingPlan, seed: u64) -> Result<Vec<Vec<u8>>, Error> {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    let chunks = abf.len().div_ceil(plan.chunk_size);
    let workers = worker_count(plan, chunks);
    let new_encoder = || Encoder::with_flags(plan.params.window_size_log2, plan.encoder_flags);
    if workers <= 1 {
        let mut encoder = new_encoder()?;
        return (0..chunks)
            .map(|i| compress_one_chunk(&mut encoder, abf, i, plan, seed))
            .collect();
    }

    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let worker = || -> Vec<(usize, Result<Vec<u8>, Error>)> {
        let mut done = Vec::new();
        let mut encoder = match new_encoder() {
            Ok(e) => e,
            Err(e) => {
                failed.store(true, Ordering::Relaxed);
                done.push((usize::MAX, Err(e)));
                return done;
            }
        };
        while !failed.load(Ordering::Relaxed) {
            let i = next.fetch_add(1, Ordering::Relaxed);
            if i >= chunks {
                break;
            }
            let r = compress_one_chunk(&mut encoder, abf, i, plan, seed);
            if r.is_err() {
                failed.store(true, Ordering::Relaxed);
            }
            done.push((i, r));
        }
        done
    };

    let mut slots: Vec<Option<Vec<u8>>> = Vec::new();
    slots
        .try_reserve_exact(chunks)
        .map_err(|_| Error::LimitExceeded(abf.len()))?;
    slots.resize_with(chunks, || None);
    let mut first_error: Option<(usize, Error)> = None;
    std::thread::scope(|scope| {
        // the calling thread is one of the workers; a thread that cannot be spawned only
        // leaves more chunks to the others
        let handles: Vec<_> = (1..workers)
            .filter_map(|_| std::thread::Builder::new().spawn_scoped(scope, worker).ok())
            .collect();
        let mut all = vec![worker()];
        for handle in handles {
            all.push(
                handle
                    .join()
                    .unwrap_or_else(|_| vec![(usize::MAX, Err(Error::Internal("encoder thread panicked")))]),
            );
        }
        for results in all {
            for (i, r) in results {
                match r {
                    Ok(payload) => {
                        if let Some(slot) = slots.get_mut(i) {
                            *slot = Some(payload);
                        }
                    }
                    // report the error of the lowest chunk, independently of scheduling
                    Err(e) => {
                        if first_error.as_ref().is_none_or(|(j, _)| i < *j) {
                            first_error = Some((i, e));
                        }
                    }
                }
            }
        }
    });
    if let Some((_, e)) = first_error {
        return Err(e);
    }
    slots
        .into_iter()
        .map(|s| s.ok_or(Error::Internal("a chunk was not compressed")))
        .collect()
}
