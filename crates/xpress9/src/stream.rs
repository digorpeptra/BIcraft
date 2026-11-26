//! Decoding a `DataModel` part as a stream, for parts (or backups) too large to hold in memory
//! (`docs/formats/xpress9.md`, "Streaming"). The framing is the one [`crate::datamodel`]
//! describes; the part is read front to back once, one chunk at a time, and each decoded chunk
//! is handed to a sink with its offset in the backup. Memory stays at one chunk plus the
//! decoder's history window, whatever the part's size.

use std::io::Read;
use std::sync::Arc;

use crate::datamodel::{BANNER_SIZE, CHUNK_HEADER_SIZE, Layout, MULTITHREADED_HEADER_SIZE, detect_layout};
use crate::dec_lz77::Decoder;
use crate::{Error, WINDOW_SIZE_LOG2_MAX};

/// Largest chunk envelope accepted (compressed or decompressed size): 256 MiB. Desktop writes
/// 2 MiB chunks; the bound keeps a hostile envelope from allocating more.
pub const MAX_STREAM_CHUNK: usize = 256 << 20;

/// Piece size used to pass an uncompressed (bare ABF) part through.
const PASS_THROUGH: usize = 1 << 20;

/// How a streamed decode ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamEnd {
    pub layout: Layout,
    /// Decoded bytes handed to the sink.
    pub decoded: u64,
    /// Whether the sink asked to stop before the end of the part.
    pub stopped: bool,
}

fn read_error(e: std::io::Error) -> Error {
    Error::Read(e.to_string())
}

/// Reads until `buf` is full or the input ends; returns the bytes read.
fn fill<R: Read + ?Sized>(r: &mut R, buf: &mut [u8]) -> Result<usize, Error> {
    let mut n = 0;
    while let Some(rest) = buf.get_mut(n..) {
        if rest.is_empty() {
            break;
        }
        match r.read(rest) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(read_error(e)),
        }
    }
    Ok(n)
}

fn u32_le(b: &[u8], at: usize) -> Result<u32, Error> {
    b.get(at..)
        .and_then(|s| s.first_chunk::<4>())
        .map(|w| u32::from_le_bytes(*w))
        .ok_or(Error::Internal("envelope"))
}

fn u64_le(b: &[u8], at: usize) -> Result<u64, Error> {
    b.get(at..)
        .and_then(|s| s.first_chunk::<8>())
        .map(|w| u64::from_le_bytes(*w))
        .ok_or(Error::Truncated("multithreaded header"))
}

/// The chunk decoder: reads envelopes and payloads, decodes, feeds the sink.
struct Chunks<'s, R> {
    reader: R,
    decoder: Decoder,
    payload: Vec<u8>,
    out: Vec<u8>,
    offset: u64,
    sink: &'s mut dyn FnMut(u64, &[u8]) -> bool,
}

impl<R: Read> Chunks<'_, R> {
    /// Decodes the next chunk. `Ok(None)` at a clean end of input (only where `end_ok`),
    /// `Ok(Some(false))` when the sink asked to stop.
    fn next(&mut self, end_ok: bool) -> Result<Option<bool>, Error> {
        let mut env = [0u8; CHUNK_HEADER_SIZE];
        let got = fill(&mut self.reader, &mut env)?;
        if got == 0 && end_ok {
            return Ok(None);
        }
        if got < CHUNK_HEADER_SIZE {
            return Err(Error::Framing("truncated chunk header"));
        }
        let uncompressed = usize::try_from(u32_le(&env, 0)?).map_err(|_| Error::Framing("chunk size"))?;
        let compressed = usize::try_from(u32_le(&env, 4)?).map_err(|_| Error::Framing("chunk size"))?;
        if uncompressed > MAX_STREAM_CHUNK || compressed > MAX_STREAM_CHUNK {
            return Err(Error::LimitExceeded(MAX_STREAM_CHUNK));
        }
        self.payload.clear();
        self.payload
            .try_reserve(compressed)
            .map_err(|_| Error::LimitExceeded(compressed))?;
        self.payload.resize(compressed, 0);
        if fill(&mut self.reader, &mut self.payload)? < compressed {
            return Err(Error::Framing("chunk extends past the end of the part"));
        }
        self.out.clear();
        let produced = self
            .decoder
            .decode_blocks(&self.payload, &mut self.out, uncompressed)?;
        if produced != uncompressed {
            return Err(Error::SizeMismatch {
                expected: uncompressed,
                actual: produced,
            });
        }
        let go_on = (self.sink)(self.offset, &self.out);
        self.offset = self
            .offset
            .checked_add(u64::try_from(produced).map_err(|_| Error::Internal("size"))?)
            .ok_or(Error::Framing("backup size overflow"))?;
        Ok(Some(go_on))
    }
}

/// Decodes the `DataModel` part read from `reader`, front to back, calling `sink(offset, bytes)`
/// for each decoded piece in order (`offset` is the position of `bytes` in the backup). The sink
/// returns `false` to stop early. A part that is a bare ABF is passed through unchanged.
///
/// Every chunk is checked as in [`crate::decompress_datamodel_reference`]; the sizes are bounded
/// by [`MAX_STREAM_CHUNK`] per chunk, not in total, so a caller that must bound the work bounds
/// the input.
pub fn stream_datamodel<R: Read>(
    mut reader: R,
    sink: &mut dyn FnMut(u64, &[u8]) -> bool,
) -> Result<StreamEnd, Error> {
    let mut banner = vec![0u8; BANNER_SIZE];
    let got = fill(&mut reader, &mut banner)?;
    banner.truncate(got);
    let layout = detect_layout(&banner)?;
    if layout == Layout::Uncompressed {
        let mut offset = 0u64;
        let mut piece = banner;
        let mut buf = vec![0u8; PASS_THROUGH];
        loop {
            if !piece.is_empty() {
                let len = u64::try_from(piece.len()).map_err(|_| Error::Internal("size"))?;
                if !sink(offset, &piece) {
                    return Ok(StreamEnd {
                        layout,
                        decoded: offset.saturating_add(len),
                        stopped: true,
                    });
                }
                offset = offset.saturating_add(len);
            }
            let n = fill(&mut reader, &mut buf)?;
            if n == 0 {
                return Ok(StreamEnd {
                    layout,
                    decoded: offset,
                    stopped: false,
                });
            }
            piece.clear();
            piece.extend_from_slice(buf.get(..n).unwrap_or_default());
        }
    }
    let mut chunks = Chunks {
        reader,
        decoder: Decoder::new(WINDOW_SIZE_LOG2_MAX)?,
        payload: Vec::new(),
        out: Vec::new(),
        offset: 0,
        sink,
    };
    let stop = |chunks: &Chunks<'_, R>| StreamEnd {
        layout,
        decoded: chunks.offset,
        stopped: true,
    };
    match layout {
        Layout::SingleThreaded => {
            while let Some(go_on) = chunks.next(true)? {
                if !go_on {
                    return Ok(stop(&chunks));
                }
            }
        }
        _ => {
            let mut h = [0u8; MULTITHREADED_HEADER_SIZE];
            if fill(&mut chunks.reader, &mut h)? < MULTITHREADED_HEADER_SIZE {
                return Err(Error::Truncated("multithreaded header"));
            }
            // main chunks per thread, prefix chunks per thread, prefix threads, main threads
            let groups = [
                (u64_le(&h, 16)?, u64_le(&h, 8)?),
                (u64_le(&h, 24)?, u64_le(&h, 0)?),
            ];
            for (threads, per_thread) in groups {
                if threads == 0 || per_thread == 0 {
                    continue;
                }
                // Each chunk reads at least an envelope, so these loops end with the input.
                for _ in 0..threads {
                    chunks.decoder.start_session();
                    for _ in 0..per_thread {
                        match chunks.next(false)? {
                            Some(true) => {}
                            Some(false) => return Ok(stop(&chunks)),
                            None => return Err(Error::Framing("truncated chunk header")),
                        }
                    }
                }
            }
        }
    }
    Ok(StreamEnd {
        layout,
        decoded: chunks.offset,
        stopped: false,
    })
}

/// Where [`decompress_datamodel_into`] hands the backup: (offset in the backup, bytes); an error
/// stops the decode. It is called from a few threads at once, beside the decoding, so the pieces
/// may come out of order: each goes to its offset.
pub type BackupSink<'s> = dyn Fn(u64, &[u8]) -> Result<(), Error> + Sync + 's;

/// Compressed bytes of the input read per piece by [`decompress_datamodel_into`].
pub const STREAM_PIECE: usize = 8 << 20;

/// Bytes of output per job of [`decompress_datamodel_into`]'s workers.
const STREAM_JOB: usize = 1 << 20;

/// Decoded jobs waiting for the sink.
const WRITE_QUEUE: usize = 8;

/// Threads handing the decoded jobs to the sink.
const WRITERS: usize = 3;

/// About the most bytes [`decompress_datamodel_into_lean`] holds: the window, a chunk read and a chunk
/// decoded (Desktop writes chunks of 2 MiB), with room for the decoder's state.
pub const LEAN_BUFFER_BYTES: usize = (1 << crate::WINDOW_SIZE_LOG2_MAX) + (6 << 20);

/// About the most bytes [`decompress_datamodel_into_with`] holds at once with `threads` workers
/// (0: automatic) and pieces of `piece` compressed bytes: three pieces (one read ahead, one queued,
/// one decoding), each with a chunk past it, two jobs per worker, the queued and written jobs, and
/// the window. An estimate for memory limits (`docs/beyond-ram.md`, lot 5); a hostile part's chunks
/// are bounded apart ([`MAX_STREAM_CHUNK`]).
pub fn stream_buffer_bytes(threads: usize, piece: usize) -> usize {
    let t = crate::dec_par::thread_count(threads).max(1);
    let chunk = 2usize << 20;
    let pieces = piece.max(1).saturating_add(chunk).saturating_mul(3);
    let jobs = t
        .saturating_mul(2)
        .saturating_add(WRITE_QUEUE + WRITERS + 1)
        .saturating_mul(STREAM_JOB);
    let window = 1usize << crate::WINDOW_SIZE_LOG2_MAX;
    pieces.saturating_add(jobs).saturating_add(window)
}

/// The decoded jobs waiting for the sink: (offset, bytes), shared by the writer threads.
type JobQueue = Arc<std::sync::Mutex<std::sync::mpsc::Receiver<(usize, Arc<Vec<u8>>)>>>;

/// Decompresses the `DataModel` part (or `cache.abf`) that `open` reads, front to back, on
/// several threads, handing the backup to `sink(offset, bytes)`, without holding the
/// part or the backup in memory. Three stages run at once: a thread reads the part a piece of
/// [`STREAM_PIECE`] compressed bytes ahead, the workers decode the current piece (a bounded number
/// of jobs ahead of the calling thread, which finalises them in order against the XPress9
/// window), and a few threads hand the finished jobs to the sink (a few queued at most), each at
/// its offset, not necessarily in order.
/// Returns the backup's size. More than `limit` decompressed bytes is an error.
///
/// The bytes and errors are those of [`crate::decompress_datamodel`]: when the fast decoder
/// cannot vouch for a piece, the part is read again from the start (`open` is called again) by
/// the reference streaming decoder ([`stream_datamodel`]), whose pieces overwrite what the sink
/// was given (same offsets). A sink error stops the decode and is returned.
pub fn decompress_datamodel_into<R: Read + Send>(
    open: &mut dyn FnMut() -> Result<R, Error>,
    limit: u64,
    sink: &BackupSink<'_>,
) -> Result<u64, Error> {
    decompress_datamodel_into_with(open, limit, 0, STREAM_PIECE, sink)
}

/// [`decompress_datamodel_into`] on at most `threads` threads (0: automatic) with pieces of
/// `piece` compressed bytes (tests use small ones).
pub fn decompress_datamodel_into_with<R: Read + Send>(
    open: &mut dyn FnMut() -> Result<R, Error>,
    limit: u64,
    threads: usize,
    piece: usize,
    sink: &BackupSink<'_>,
) -> Result<u64, Error> {
    match fast_into(open()?, limit, threads, piece.max(1), sink) {
        Ok(Some(n)) => return Ok(n),
        Ok(None) => {}
        Err(e) => return Err(e),
    }
    // the reference decoder from the start: its bytes and errors
    decompress_datamodel_into_lean(open, limit, sink)
}

/// [`decompress_datamodel_into`] by the reference streaming decoder alone, on the calling thread:
/// slower, but it holds only the decoder's window and one chunk in and out ([`LEAN_BUFFER_BYTES`]),
/// for tight memory limits (`docs/beyond-ram.md`, lot 5). Same bytes and errors.
pub fn decompress_datamodel_into_lean<R: Read>(
    open: &mut dyn FnMut() -> Result<R, Error>,
    limit: u64,
    sink: &BackupSink<'_>,
) -> Result<u64, Error> {
    let mut failed: Option<Error> = None;
    let end = stream_datamodel(open()?, &mut |at, bytes| {
        let over = at
            .checked_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
            .is_none_or(|e| e > limit);
        if over {
            failed = Some(Error::LimitExceeded(usize::try_from(limit).unwrap_or(usize::MAX)));
            return false;
        }
        match sink(at, bytes) {
            Ok(()) => true,
            Err(e) => {
                failed = Some(e);
                false
            }
        }
    })?;
    match failed {
        Some(e) => Err(e),
        None => Ok(end.decoded),
    }
}

/// Why the fast path stopped: it cannot vouch for the result (the reference decoder takes
/// over), or an error to return as is.
enum Stop {
    Fast,
    Err(Error),
}

/// Where a part read as a stream is in its framing: the session of the next chunk.
struct Framing {
    layout: Layout,
    /// (threads, chunks per thread) of each group of a multithreaded part.
    sessions: Vec<(u64, u64)>,
    group: usize,
    /// (thread of the group, chunk of the thread) of the next chunk.
    in_group: (u64, u64),
    session_id: usize,
    declared: u64,
    done: bool,
}

/// One piece of the part: the chunks' payloads, and per chunk (session, range in `buf`,
/// decompressed size).
struct Piece {
    buf: Vec<u8>,
    units: Vec<(usize, std::ops::Range<usize>, usize)>,
}

/// Reads the banner (and the multithreaded header).
fn read_head<R: Read>(reader: &mut R) -> Result<Framing, Stop> {
    let mut banner = vec![0u8; BANNER_SIZE];
    let got = fill(reader, &mut banner).map_err(|_| Stop::Fast)?;
    banner.truncate(got);
    let layout = detect_layout(&banner).map_err(|_| Stop::Fast)?;
    if layout == Layout::Uncompressed {
        // passed through by the reference
        return Err(Stop::Fast);
    }
    let mut sessions: Vec<(u64, u64)> = Vec::new();
    if layout == Layout::MultiThreaded {
        let mut h = [0u8; MULTITHREADED_HEADER_SIZE];
        if !matches!(fill(reader, &mut h), Ok(n) if n == MULTITHREADED_HEADER_SIZE) {
            return Err(Stop::Fast);
        }
        let (Ok(a), Ok(b), Ok(c), Ok(d)) = (u64_le(&h, 0), u64_le(&h, 8), u64_le(&h, 16), u64_le(&h, 24))
        else {
            return Err(Stop::Fast);
        };
        // main chunks per thread, prefix chunks per thread, prefix threads, main threads
        for (threads, per_thread) in [(c, b), (d, a)] {
            if threads > 0 && per_thread > 0 {
                sessions.push((threads, per_thread));
            }
        }
    }
    Ok(Framing {
        layout,
        sessions,
        group: 0,
        in_group: (0, 0),
        session_id: 0,
        declared: 0,
        done: false,
    })
}

/// The next piece: chunks until `piece` compressed bytes (or the end); `None` at the end.
fn next_piece<R: Read>(
    reader: &mut R,
    f: &mut Framing,
    piece: usize,
    limit: u64,
) -> Result<Option<Piece>, Stop> {
    let mut buf: Vec<u8> = Vec::new();
    let mut units = Vec::new();
    while !f.done && buf.len() < piece {
        if f.layout == Layout::MultiThreaded {
            let Some(&(threads, per_thread)) = f.sessions.get(f.group) else {
                f.done = true;
                break;
            };
            if f.in_group.1 == per_thread {
                f.in_group = (f.in_group.0 + 1, 0);
                f.session_id += 1;
            }
            if f.in_group.0 == threads {
                f.group += 1;
                f.in_group = (0, 0);
                continue;
            }
            f.in_group.1 += 1;
        }
        let mut env = [0u8; CHUNK_HEADER_SIZE];
        let got = fill(reader, &mut env).map_err(|_| Stop::Fast)?;
        if got == 0 && f.layout == Layout::SingleThreaded {
            f.done = true;
            break;
        }
        if got < CHUNK_HEADER_SIZE {
            return Err(Stop::Fast);
        }
        let (Ok(uncompressed), Ok(compressed)) = (u32_le(&env, 0), u32_le(&env, 4)) else {
            return Err(Stop::Fast);
        };
        let (uncompressed, compressed) = (uncompressed as usize, compressed as usize);
        if uncompressed > MAX_STREAM_CHUNK || compressed > MAX_STREAM_CHUNK {
            return Err(Stop::Fast);
        }
        f.declared = f.declared.saturating_add(uncompressed as u64);
        if f.declared > limit {
            return Err(Stop::Err(Error::LimitExceeded(
                usize::try_from(limit).unwrap_or(usize::MAX),
            )));
        }
        let start = buf.len();
        buf.try_reserve(compressed).map_err(|_| Stop::Fast)?;
        buf.resize(start + compressed, 0);
        let dst = buf.get_mut(start..).ok_or(Stop::Fast)?;
        if !matches!(fill(reader, dst), Ok(n) if n == compressed) {
            return Err(Stop::Fast);
        }
        units.push((f.session_id, start..start + compressed, uncompressed));
    }
    Ok((!units.is_empty()).then_some(Piece { buf, units }))
}

/// The fast path of [`decompress_datamodel_into_with`]: `Ok(None)` when it cannot vouch for
/// the result (the caller then runs the reference decoder), `Err` for a sink error or a limit
/// exceeded (the declared sizes say so before anything of the piece is decoded).
fn fast_into<R: Read + Send>(
    mut reader: R,
    limit: u64,
    threads: usize,
    piece: usize,
    sink: &BackupSink<'_>,
) -> Result<Option<u64>, Error> {
    let mut framing = match read_head(&mut reader) {
        Ok(f) => f,
        Err(Stop::Fast) => return Ok(None),
        Err(Stop::Err(e)) => return Err(e),
    };
    let sink_stopped = std::sync::atomic::AtomicBool::new(false);
    let write = |jobs: JobQueue| -> Result<(), Error> {
        loop {
            let next = match jobs.lock() {
                Ok(rx) => rx.recv(),
                Err(_) => return Err(Error::Internal("the sink's queue failed")),
            };
            let Ok((at, bytes)) = next else { return Ok(()) };
            if let Err(e) = sink(at as u64, &bytes) {
                sink_stopped.store(true, std::sync::atomic::Ordering::Relaxed);
                return Err(e);
            }
            if sink_stopped.load(std::sync::atomic::Ordering::Relaxed) {
                return Ok(());
            }
        }
    };
    std::thread::scope(|scope| {
        // the part read a piece ahead
        let (piece_tx, pieces) = std::sync::mpsc::sync_channel::<Result<Piece, Stop>>(1);
        let reading = std::thread::Builder::new().spawn_scoped(scope, move || {
            loop {
                match next_piece(&mut reader, &mut framing, piece, limit) {
                    Ok(Some(p)) => {
                        if piece_tx.send(Ok(p)).is_err() {
                            return;
                        }
                    }
                    Ok(None) => return,
                    Err(s) => {
                        let _ = piece_tx.send(Err(s));
                        return;
                    }
                }
            }
        });
        // the finished jobs to the sink, on threads of their own (each job at its offset)
        let (job_tx, jobs) = std::sync::mpsc::sync_channel::<(usize, Arc<Vec<u8>>)>(WRITE_QUEUE);
        // each writer holds the queue: when the last one stops (a sink error), sending fails
        let jobs = Arc::new(std::sync::Mutex::new(jobs));
        let writers: Vec<_> = (0..WRITERS)
            .filter_map(|_| {
                let jobs = Arc::clone(&jobs);
                let write = &write;
                std::thread::Builder::new()
                    .spawn_scoped(scope, move || write(jobs))
                    .ok()
            })
            .collect();
        drop(jobs);
        let Ok(reading) = reading else {
            return Ok(None);
        };
        if writers.is_empty() {
            drop(job_tx);
            return Ok(None);
        }
        let outcome = (|| -> Result<Option<u64>, Error> {
            let mut state = crate::dec_fast::PlanState::default();
            let mut next = || -> Result<Option<crate::dec_par::OwnedPiece>, crate::dec_par::PieceError> {
                use crate::dec_par::PieceError;
                let p = match pieces.recv() {
                    Ok(Ok(p)) => p,
                    Ok(Err(Stop::Fast)) => return Err(PieceError::Fast),
                    Ok(Err(Stop::Err(e))) => return Err(PieceError::Stop(e)),
                    // the reader ended
                    Err(_) => return Ok(None),
                };
                let parts = {
                    let refs: Option<Vec<crate::dec_fast::Unit<'_>>> = p
                        .units
                        .iter()
                        .map(|(s, r, n)| p.buf.get(r.clone()).map(|b| (*s, b, *n)))
                        .collect();
                    let refs = refs.ok_or(PieceError::Fast)?;
                    let (plan, after) = crate::dec_fast::plan_from(&refs, crate::WINDOW_SIZE_LOG2_MAX, state)
                        .ok_or(PieceError::Fast)?;
                    state = after;
                    crate::dec_par::OwnedPiece::layout(&p.buf, &plan, STREAM_JOB).ok_or(PieceError::Fast)?
                };
                Ok(Some(crate::dec_par::OwnedPiece::from_parts(p.buf, parts)))
            };
            let mut to_sink = |at: usize, bytes: &Arc<Vec<u8>>| {
                job_tx
                    .send((at, Arc::clone(bytes)))
                    .map_err(|_| Error::Write("the sink stopped".into()))
            };
            match crate::dec_par::decode_stream(&mut next, threads, &mut to_sink) {
                Ok(()) => {}
                Err(crate::dec_par::PieceError::Fast) => return Ok(None),
                // a sink error: the sink's own error is returned below
                Err(crate::dec_par::PieceError::Stop(e)) => return Err(e),
            }
            Ok(Some(state.out_pos as u64))
        })();
        drop(job_tx);
        drop(pieces);
        let mut written = Ok(());
        for w in writers {
            let r = w.join().unwrap_or(Err(Error::Internal("a sink thread failed")));
            if written.is_ok() {
                written = r;
            }
        }
        let read_ok = reading.join().is_ok();
        match (outcome, written) {
            (_, Err(e)) => Err(e),
            (Ok(Some(_)), Ok(())) if !read_ok => Ok(None),
            (o, Ok(())) => o,
        }
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::{FramingPlan, compress_datamodel, datamodel::compress_with_plan, decompress_datamodel};

    fn sample(len: usize) -> Vec<u8> {
        let mut x = 0x1234_5678u32;
        (0..len)
            .map(|i| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                if i % 7 < 4 {
                    (i / 13) as u8
                } else {
                    (x >> 24) as u8
                }
            })
            .collect()
    }

    fn collect(part: &[u8]) -> (Vec<u8>, StreamEnd) {
        let mut out = Vec::new();
        let end = stream_datamodel(part, &mut |at, b| {
            assert_eq!(at as usize, out.len());
            out.extend_from_slice(b);
            true
        })
        .unwrap();
        (out, end)
    }

    #[test]
    fn the_lean_decoder_writes_the_same_backup() {
        for len in [3 << 20, 100_000] {
            let abf = sample(len);
            let part = compress_datamodel(&abf, None).unwrap();
            let out = std::sync::Mutex::new(vec![0u8; abf.len()]);
            let mut open = || -> Result<&[u8], Error> { Ok(&part[..]) };
            let n = decompress_datamodel_into_lean(&mut open, u64::MAX, &|at, b| {
                let mut o = out.lock().unwrap();
                o[at as usize..at as usize + b.len()].copy_from_slice(b);
                Ok(())
            })
            .unwrap();
            assert_eq!(n, abf.len() as u64);
            assert_eq!(out.into_inner().unwrap(), abf);
            assert!(decompress_datamodel_into_lean(&mut open, 1000, &|_, _| Ok(())).is_err());
        }
        assert!(LEAN_BUFFER_BYTES < stream_buffer_bytes(1, 1 << 20));
    }

    #[test]
    fn streams_the_same_bytes_as_the_whole_part_decoder() {
        let abf = sample(5 << 20);
        let part = compress_datamodel(&abf, None).unwrap();
        let (out, end) = collect(&part);
        assert_eq!(out, decompress_datamodel(&part, abf.len()).unwrap());
        assert_eq!(out, abf);
        assert_eq!(end.decoded, abf.len() as u64);
        assert!(!end.stopped);
        // Small chunks: history must carry across them.
        let mut plan = FramingPlan::new(None);
        plan.chunk_size = 64 << 10;
        let part = compress_with_plan(&abf, &plan.with_mode(crate::CompressionMode::Exact)).unwrap();
        assert_eq!(collect(&part).0, abf);
    }

    #[test]
    fn a_bare_backup_passes_through_and_the_sink_can_stop() {
        let mut abf = vec![0xFF, 0xFE];
        abf.extend(
            "STREAM_STORAGE_SIGNATURE_)!@#$%^&*("
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        );
        abf.extend(sample(3 << 20));
        let (out, end) = collect(&abf);
        assert_eq!(out, abf);
        assert_eq!(end.layout, Layout::Uncompressed);
        let part = compress_datamodel(&sample(6 << 20), None).unwrap();
        let mut seen = 0usize;
        let end = stream_datamodel(part.as_slice(), &mut |_, b| {
            seen += b.len();
            false
        })
        .unwrap();
        assert!(end.stopped);
        assert_eq!(end.decoded, seen as u64);
        assert!(seen < 6 << 20);
    }

    /// The multithreaded layout: each thread group is its own session.
    #[test]
    fn streams_the_multithreaded_layout() {
        let (a, b) = (sample(200 << 10), sample(150 << 10));
        let mut plan = FramingPlan::new(None);
        plan.chunk_size = 64 << 10;
        let part_a = compress_with_plan(&a[..128 << 10], &plan).unwrap();
        let part_b = compress_with_plan(&b[..128 << 10], &plan).unwrap();
        let banner: Vec<u8> = format!("{:30}multithreaded", "")
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .chain(std::iter::repeat(0))
            .take(BANNER_SIZE)
            .collect();
        let mut mt = banner;
        for v in [2u64, 0, 0, 2, 64 << 10] {
            mt.extend_from_slice(&v.to_le_bytes());
        }
        mt.extend_from_slice(&part_a[BANNER_SIZE..]);
        mt.extend_from_slice(&part_b[BANNER_SIZE..]);
        let expected = [&a[..128 << 10], &b[..128 << 10]].concat();
        assert_eq!(decompress_datamodel(&mt, 1 << 20).unwrap(), expected);
        let (out, end) = collect(&mt);
        assert_eq!(out, expected);
        assert_eq!(end.layout, Layout::MultiThreaded);
        assert!(stream_datamodel(&mt[..mt.len() - 10], &mut |_, _| true).is_err());
    }

    #[test]
    fn broken_parts_are_errors() {
        let part = compress_datamodel(&sample(1 << 20), None).unwrap();
        for cut in [10, BANNER_SIZE + 3, BANNER_SIZE + 20, part.len() - 1] {
            assert!(
                stream_datamodel(&part[..cut], &mut |_, _| true).is_err(),
                "cut {cut}"
            );
        }
        let mut huge = part.clone();
        huge[BANNER_SIZE..BANNER_SIZE + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            stream_datamodel(huge.as_slice(), &mut |_, _| true),
            Err(Error::LimitExceeded(_))
        ));
        let mut wrong = part.clone();
        wrong[BANNER_SIZE..BANNER_SIZE + 4].copy_from_slice(&1000u32.to_le_bytes());
        assert!(stream_datamodel(wrong.as_slice(), &mut |_, _| true).is_err());
    }

    /// Decodes with [`decompress_datamodel_into_with`] into a vector (writes at the sink's
    /// offsets, as a file would take them).
    fn into_vec(part: &[u8], limit: u64, threads: usize, piece: usize) -> Result<Vec<u8>, Error> {
        let out = std::sync::Mutex::new(Vec::<u8>::new());
        let mut opens = 0;
        let n = decompress_datamodel_into_with(
            &mut || {
                opens += 1;
                Ok(part)
            },
            limit,
            threads,
            piece,
            &|at, b| {
                let at = at as usize;
                let mut out = out.lock().unwrap();
                if out.len() < at + b.len() {
                    out.resize(at + b.len(), 0);
                }
                out[at..at + b.len()].copy_from_slice(b);
                Ok(())
            },
        )?;
        let out = out.into_inner().unwrap();
        assert_eq!(n as usize, out.len());
        Ok(out)
    }

    #[test]
    fn streamed_threads_give_the_whole_part_decoders_bytes() {
        let abf = sample(3 << 20);
        let mut plan = FramingPlan::new(None);
        plan.chunk_size = 64 << 10;
        for mode in [crate::CompressionMode::Exact, crate::CompressionMode::Fast] {
            let part = compress_with_plan(&abf, &plan.clone().with_mode(mode)).unwrap();
            for threads in [1, 4] {
                for piece in [1, 100 << 10, 1 << 20, 64 << 20] {
                    assert_eq!(
                        into_vec(&part, 1 << 30, threads, piece).unwrap(),
                        abf,
                        "{mode:?} threads {threads} piece {piece}"
                    );
                }
            }
        }
        // a bare backup passes through (the reference path)
        let mut bare = vec![0xFF, 0xFE];
        bare.extend(
            "STREAM_STORAGE_SIGNATURE_)!@#$%^&*("
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        );
        bare.extend(sample(100 << 10));
        assert_eq!(into_vec(&bare, 1 << 30, 4, 1 << 10).unwrap(), bare);
    }

    #[test]
    fn streamed_threads_read_the_multithreaded_layout() {
        let (a, b) = (sample(200 << 10), sample(150 << 10));
        let mut plan = FramingPlan::new(None);
        plan.chunk_size = 64 << 10;
        let part_a = compress_with_plan(&a[..128 << 10], &plan).unwrap();
        let part_b = compress_with_plan(&b[..128 << 10], &plan).unwrap();
        let mut mt: Vec<u8> = format!("{:30}multithreaded", "")
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .chain(std::iter::repeat(0))
            .take(BANNER_SIZE)
            .collect();
        for v in [2u64, 0, 0, 2, 64 << 10] {
            mt.extend_from_slice(&v.to_le_bytes());
        }
        mt.extend_from_slice(&part_a[BANNER_SIZE..]);
        mt.extend_from_slice(&part_b[BANNER_SIZE..]);
        let expected = [&a[..128 << 10], &b[..128 << 10]].concat();
        for (threads, piece) in [(1, 1), (4, 1), (4, 50 << 10), (3, 1 << 20)] {
            assert_eq!(into_vec(&mt, 1 << 30, threads, piece).unwrap(), expected);
        }
        assert_eq!(
            into_vec(&mt[..mt.len() - 10], 1 << 30, 4, 1 << 10).unwrap_err(),
            decompress_datamodel(&mt[..mt.len() - 10], 1 << 30).unwrap_err()
        );
    }

    #[test]
    fn streamed_threads_give_the_same_errors_and_stop_on_sink_errors() {
        let abf = sample(1 << 20);
        let part = compress_datamodel(&abf, None).unwrap();
        // limits: refused from the declared sizes
        assert!(matches!(
            into_vec(&part, abf.len() as u64 - 1, 4, 1 << 10),
            Err(Error::LimitExceeded(_))
        ));
        assert_eq!(into_vec(&part, abf.len() as u64, 4, 1 << 10).unwrap(), abf);
        for cut in [10, BANNER_SIZE + 3, BANNER_SIZE + 20, part.len() - 1] {
            assert!(into_vec(&part[..cut], 1 << 30, 4, 1 << 10).is_err(), "cut {cut}");
        }
        let mut broken = part.clone();
        let mid = broken.len() / 2;
        broken[mid] ^= 0x5A;
        assert_eq!(
            into_vec(&broken, 1 << 30, 4, 1 << 10).map_err(|e| e.to_string()),
            decompress_datamodel(&broken, 1 << 30).map_err(|e| e.to_string())
        );
        // the sink's error stops the decode
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let e = decompress_datamodel_into_with(&mut || Ok(part.as_slice()), 1 << 30, 4, 1 << 10, &|_, _| {
            if calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0 {
                Err(Error::Write("disk full".into()))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(e, Error::Write("disk full".into()));
    }
}
