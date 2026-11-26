// Ported from Microsoft XPress9 (MIT): the window bounds and status codes of
// dev/ese/src/_xpress9/xpress9.h@7030fe7407615160e54d152e4ef704eede2fdd7e
// Copyright (c) Microsoft Corporation. Licensed under the MIT License.
// See LICENSE-MIT-microsoft-xpress9 in this crate and THIRD_PARTY_NOTICES.md at the repository root.

//! L0: XPress9 codec, a pure-Rust port of Microsoft's MIT-licensed XPress9
//! (`microsoft/Extensible-Storage-Engine`, `dev/ese/src/_xpress9`, commit
//! `7030fe7407615160e54d152e4ef704eede2fdd7e`), plus the PBIX `DataModel` framing.
//!
//! - [`decompress`] / [`compress`]: one XPress9 stream (a session of one or more blocks).
//!   Decoding runs the fast decoder, on several threads for large inputs, with the ported
//!   [`Decoder`] as the reference it falls back to (`docs/formats/xpress9.md` §7).
//! - [`Decoder`] / [`Encoder`]: the session-level API; history and MTF state carry across
//!   blocks and calls within a session, as upstream.
//! - [`datamodel`]: the banner + chunk framing used by PBIX `DataModel` parts and PBIP
//!   `cache.abf` files; [`decompress_datamodel`] returns the raw ABF and
//!   [`compress_datamodel`] frames an ABF in Desktop's single-threaded layout, by default with
//!   its chunks compressed in parallel ([`CompressionMode`]).
//!
//! Every input is treated as hostile: malformed data yields an [`Error`], never a panic, and
//! output sizes are capped by the caller.
//!
//! The ported files keep Microsoft's copyright notice; see `LICENSE-MIT-microsoft-xpress9` in
//! this crate and `THIRD_PARTY_NOTICES.md` at the repository root.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable,
    clippy::indexing_slicing
)]

mod dec_fast;
mod dec_huffman;
mod dec_lz77;
mod dec_par;
mod enc_huffman;
mod enc_lz77;
mod misc;
mod zobrist;

pub mod datamodel;
pub mod stream;

pub use datamodel::{
    CompressionMode, DataModelInfo, FramingPlan, Layout, compress_datamodel, decompress_datamodel,
    decompress_datamodel_reference, decompress_datamodel_with, inspect_datamodel, verify_datamodel,
    verify_datamodel_with,
};
pub use dec_lz77::{BLOCK_HEADER_SIZE, BlockHeader, Decoder};
pub use dec_par::MAX_DECODE_THREADS;
pub use enc_lz77::{Encoder, EncoderParams, FLAG_USE_SSE2};
pub use misc::{XPRESS9_MAGIC, xpress9_crc32};
pub use stream::{
    BackupSink, LEAN_BUFFER_BYTES, MAX_STREAM_CHUNK, STREAM_PIECE, StreamEnd, decompress_datamodel_into,
    decompress_datamodel_into_lean, decompress_datamodel_into_with, stream_buffer_bytes, stream_datamodel,
};

/// `XPRESS9_WINDOW_SIZE_LOG2_MIN`.
pub const WINDOW_SIZE_LOG2_MIN: u32 = 16;
/// `XPRESS9_WINDOW_SIZE_LOG2_MAX`.
pub const WINDOW_SIZE_LOG2_MAX: u32 = 22;

/// Errors of the codec and of the `DataModel` framing. The decoder variants mirror the
/// upstream `Xpress9Status_*` codes; `Internal` stands for a failed `RETAIL_ASSERT`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("bad argument: {0}")]
    BadArgument(&'static str),
    #[error("corrupted XPress9 block header: {0}")]
    CorruptedHeader(&'static str),
    #[error("corrupted XPress9 data: {0}")]
    CorruptedData(&'static str),
    #[error("XPress9 window 2^{0} exceeds the decoder maximum 2^{1}")]
    WindowTooLarge(u32, u32),
    #[error("truncated XPress9 input: {0}")]
    Truncated(&'static str),
    #[error("decompressed data would exceed the limit of {0} bytes")]
    LimitExceeded(usize),
    #[error("decompressed size {actual} differs from the expected {expected}")]
    SizeMismatch { expected: usize, actual: usize },
    #[error("unrecognised DataModel framing: {0}")]
    Framing(&'static str),
    #[error("internal consistency check failed: {0}")]
    Internal(&'static str),
    /// Reading a streamed part failed ([`stream_datamodel`]).
    #[error("reading the DataModel failed: {0}")]
    Read(String),
    /// The sink of a streamed decode refused the bytes ([`decompress_datamodel_into`]).
    #[error("writing the decompressed DataModel failed: {0}")]
    Write(String),
}

/// Decompress one XPress9 stream (all blocks of one session) that must expand to exactly
/// `expected_len` bytes; `expected_len` is also the output cap.
///
/// Uses the fast decoder (on up to [`MAX_DECODE_THREADS`] threads), falling back to
/// [`decompress_reference`] for anything it does not vouch for, so results and errors are the
/// reference decoder's.
pub fn decompress(input: &[u8], expected_len: usize) -> Result<Vec<u8>, Error> {
    if let Some(out) = dec_par::decode(&[(0, input, expected_len)], WINDOW_SIZE_LOG2_MAX, 0) {
        return Ok(out);
    }
    decompress_reference(input, expected_len)
}

/// [`decompress`] with the reference decoder only: one [`Decoder`] session on the calling
/// thread.
pub fn decompress_reference(input: &[u8], expected_len: usize) -> Result<Vec<u8>, Error> {
    let mut decoder = Decoder::new(WINDOW_SIZE_LOG2_MAX)?;
    let mut out = Vec::new();
    out.try_reserve(expected_len.min(64 << 20))
        .map_err(|_| Error::LimitExceeded(expected_len))?;
    let produced = decoder.decode_blocks(input, &mut out, expected_len)?;
    if produced != expected_len {
        return Err(Error::SizeMismatch {
            expected: expected_len,
            actual: produced,
        });
    }
    Ok(out)
}

/// Default session seed for [`compress`]; the session signature is derived from it.
pub const DEFAULT_SESSION_SEED: u64 = 0x0123_4567_89ab_cdef;

/// Compress `input` as one XPress9 session with the given parameters (one or more blocks).
pub fn compress(input: &[u8], params: &EncoderParams) -> Result<Vec<u8>, Error> {
    let mut encoder = Encoder::new(params.window_size_log2.max(WINDOW_SIZE_LOG2_MIN))?;
    encoder.start_session(params, DEFAULT_SESSION_SEED)?;
    encoder.compress(input, true)
}
