//! Driving the fast decoder ([`crate::dec_fast`]) over a whole input: on the calling thread, or
//! on up to [`MAX_DECODE_THREADS`] threads, either to produce the output or to check that an
//! input decodes to given bytes. Original bicraft code.
//!
//! Every function returns `None` when the fast path cannot vouch for the result (a malformed
//! or unusual stream, an allocation failure, a thread that could not start); the callers then
//! run the reference decoder, so results and errors never depend on the path taken. The output
//! does not depend on the thread count or the scheduling: every byte has one correct value.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};

use crate::Error;
use crate::dec_fast::{
    BlockRef, History, Job, Pending, Plan, Scratch, Source, Unit, decode_job, plan, replay,
};

/// Most threads a decode uses (0 = automatic picks `min(available parallelism, this)`).
pub const MAX_DECODE_THREADS: usize = 32;

/// Smallest job worth a thread of its own.
const MIN_JOB: usize = 256 << 10;
/// Largest job: bounds the work redone when a deferred decode gives up.
const MAX_JOB: usize = 4 << 20;

pub(crate) fn thread_count(requested: usize) -> usize {
    if requested == 0 {
        std::thread::available_parallelism()
            .map_or(1, std::num::NonZero::get)
            .min(MAX_DECODE_THREADS)
    } else {
        requested.min(MAX_DECODE_THREADS)
    }
}

/// Jobs for `threads` workers: about four per worker, within [`MIN_JOB`]..[`MAX_JOB`].
fn jobs_for(plan: &Plan<'_>, threads: usize) -> Vec<Job> {
    let target = if threads <= 1 {
        usize::MAX
    } else {
        (plan.total / (threads * 4)).clamp(MIN_JOB, MAX_JOB)
    };
    plan.jobs(target)
}

/// Most matches a deferred decode of `job` may record before it gives up (12 bytes each: at
/// most 3/4 of the job's size).
fn pending_cap(job: &Job) -> usize {
    #[cfg(test)]
    {
        let forced = tests::FORCED_PENDING_CAP.load(Ordering::Relaxed);
        if forced != usize::MAX {
            return forced;
        }
    }
    (job.out_len / 16).max(4096)
}

/// A zeroed buffer of `n` bytes, or `None` when that much memory cannot be had. The size is
/// first reserved fallibly (and released); the buffer then comes from the allocator's zeroed
/// allocation, which takes fresh pages from the system without writing them: the workers fault
/// the pages in as they decode, in parallel, instead of the calling thread zeroing them first.
fn zeroed(n: usize) -> Option<Vec<u8>> {
    let mut probe: Vec<u8> = Vec::new();
    probe.try_reserve_exact(n).ok()?;
    drop(probe);
    Some(vec![0u8; n])
}

/// Decodes `units` (see [`Unit`]) into one vector of exactly their declared total size.
pub(crate) fn decode(units: &[Unit<'_>], max_window_log2: u32, threads: usize) -> Option<Vec<u8>> {
    let plan = plan(units, max_window_log2)?;
    let mut out = zeroed(plan.total)?;
    let threads = thread_count(threads);
    let jobs = jobs_for(&plan, threads);
    let ok = if threads <= 1 || jobs.len() <= 1 {
        decode_sequential(&plan, &jobs, &mut out).is_ok()
    } else {
        decode_parallel(&plan, &jobs, &mut out, threads.min(jobs.len())) == Some(true)
    };
    ok.then_some(out)
}

/// One job after another, each with its history known.
fn decode_sequential(plan: &Plan<'_>, jobs: &[Job], out: &mut [u8]) -> Result<(), Error> {
    let mut scratch = Scratch::new();
    for job in jobs {
        let (before, rest) = out
            .split_at_mut_checked(job.out_start)
            .ok_or(Error::Internal("job range"))?;
        let history = before
            .get(job.session_start..)
            .ok_or(Error::Internal("job range"))?;
        let parts = [history];
        let history = History::new(&parts);
        // the rest of the output is not decoded yet: copies may run over into it
        decode_job(&plan.blocks, job, Source::Known(&history), rest, &mut scratch)?;
    }
    Ok(())
}

/// What a worker hands back: the job index, its output slot and its first pass.
type Message<'o> = (usize, &'o mut [u8], Result<FirstPass, Error>);

/// A job's first pass: complete (with the matches still to replay) or abandoned.
struct FirstPass {
    pending: Vec<Pending>,
    complete: bool,
}

struct Shared<'o> {
    slots: Vec<Option<&'o mut [u8]>>,
    next: usize,
    resolved: usize,
    failed: bool,
}

/// Workers decode jobs out of order (deferred when their history is not final); the calling
/// thread finalises them in order: it replays their recorded matches, or decodes a job again
/// with its history known. Workers stay at most `2 * threads` jobs ahead, which bounds the
/// memory held by recorded matches. `Some(true)` on success.
fn decode_parallel<'o>(plan: &Plan<'_>, jobs: &[Job], out: &'o mut [u8], threads: usize) -> Option<bool> {
    let mut slots: Vec<Option<&'o mut [u8]>> = Vec::new();
    slots.try_reserve_exact(jobs.len()).ok()?;
    let mut rest = out;
    for job in jobs {
        let (slot, tail) = rest.split_at_mut_checked(job.out_len)?;
        slots.push(Some(slot));
        rest = tail;
    }
    let lookahead = threads * 2;
    let shared = Mutex::new(Shared {
        slots,
        next: 0,
        resolved: 0,
        failed: false,
    });
    let wake = Condvar::new();
    let worker = |tx: mpsc::Sender<Message<'o>>| {
        let mut scratch = Scratch::new();
        loop {
            let (index, slot) = {
                let Ok(mut guard) = shared.lock() else { return };
                loop {
                    if guard.failed || guard.next >= jobs.len() {
                        return;
                    }
                    if guard.next < guard.resolved + lookahead {
                        break;
                    }
                    guard = match wake.wait(guard) {
                        Ok(g) => g,
                        Err(_) => return,
                    };
                }
                let index = guard.next;
                guard.next += 1;
                (index, guard.slots.get_mut(index).and_then(Option::take))
            };
            let (Some(slot), Some(job)) = (slot, jobs.get(index)) else {
                if let Ok(mut guard) = shared.lock() {
                    guard.failed = true;
                }
                wake.notify_all();
                return;
            };
            let mut pending = Vec::new();
            let cap = pending_cap(job);
            let result = decode_job(
                &plan.blocks,
                job,
                Source::Deferred {
                    pending: &mut pending,
                    cap,
                },
                slot,
                &mut scratch,
            )
            .map(|complete| FirstPass { pending, complete });
            if result.is_err() {
                if let Ok(mut guard) = shared.lock() {
                    guard.failed = true;
                }
                wake.notify_all();
            }
            if tx.send((index, slot, result)).is_err() {
                return;
            }
        }
    };

    std::thread::scope(|scope| {
        let (tx, rx) = mpsc::channel::<Message<'o>>();
        let mut started = 0usize;
        for _ in 0..threads {
            let tx = tx.clone();
            if std::thread::Builder::new()
                .spawn_scoped(scope, move || worker(tx))
                .is_ok()
            {
                started += 1;
            }
        }
        drop(tx);
        let outcome = if started == 0 {
            None
        } else {
            finalise(plan, jobs, &rx, &shared, &wake)
        };
        if outcome != Some(true) {
            if let Ok(mut guard) = shared.lock() {
                guard.failed = true;
            }
            wake.notify_all();
        }
        outcome
    })
}

/// The calling thread's part of [`decode_parallel`].
fn finalise<'o>(
    plan: &Plan<'_>,
    jobs: &[Job],
    rx: &mpsc::Receiver<Message<'o>>,
    shared: &Mutex<Shared<'o>>,
    wake: &Condvar,
) -> Option<bool> {
    let mut ready: Vec<Option<Message<'o>>> = Vec::new();
    ready.try_reserve_exact(jobs.len()).ok()?;
    ready.resize_with(jobs.len(), || None);
    let mut scratch = Scratch::new();
    // the finished jobs of the current session, oldest first, trimmed to the window
    let mut history: Vec<&'o [u8]> = Vec::new();
    for (index, job) in jobs.iter().enumerate() {
        let (_, slot, first) = loop {
            if let Some(entry) = ready.get_mut(index).and_then(Option::take) {
                break entry;
            }
            let message = rx.recv().ok()?;
            let i = message.0;
            *ready.get_mut(i)? = Some(message);
        };
        if job.out_start == job.session_start {
            history.clear();
        }
        let first = first.ok()?;
        {
            let h = History::new(&history);
            if first.complete {
                replay(&first.pending, slot, &h).ok()?;
            } else {
                drop(first);
                decode_job(&plan.blocks, job, Source::Known(&h), slot, &mut scratch).ok()?;
            }
        }
        history.push(slot);
        let window = 1usize << job.params.window_size_log2;
        while history.len() > 1
            && History::new(&history).len() - history.first().map_or(0, |p| p.len()) >= window
        {
            history.remove(0);
        }
        let mut guard = shared.lock().ok()?;
        guard.resolved = index + 1;
        drop(guard);
        wake.notify_all();
    }
    Some(true)
}

/// Checks that `units` decode to `expected`, without building the output: each job is decoded
/// on its own (in parallel) with `expected` as its history and compared with its part of
/// `expected`. If every job matches, the whole input decodes to `expected` (by induction over
/// the jobs: each one sees exactly the history the real decode would give it).
/// `Some(false)` on a mismatch.
pub(crate) fn verify(
    units: &[Unit<'_>],
    expected: &[u8],
    max_window_log2: u32,
    threads: usize,
) -> Option<bool> {
    let plan = plan(units, max_window_log2)?;
    if plan.total != expected.len() {
        return Some(false);
    }
    let threads = thread_count(threads);
    let jobs = jobs_for(&plan, threads.max(2));
    let next = AtomicUsize::new(0);
    let mismatch = AtomicBool::new(false);
    let broken = AtomicBool::new(false);
    let worker = || {
        let mut scratch = Scratch::new();
        let mut buffer: Vec<u8> = Vec::new();
        while !mismatch.load(Ordering::Relaxed) && !broken.load(Ordering::Relaxed) {
            let index = next.fetch_add(1, Ordering::Relaxed);
            let Some(job) = jobs.get(index) else { return };
            match verify_job(&plan, job, expected, &mut buffer, &mut scratch) {
                Some(true) => {}
                Some(false) => mismatch.store(true, Ordering::Relaxed),
                None => broken.store(true, Ordering::Relaxed),
            }
        }
    };
    std::thread::scope(|scope| {
        let handles: Vec<_> = (1..threads.min(jobs.len()))
            .filter_map(|_| std::thread::Builder::new().spawn_scoped(scope, worker).ok())
            .collect();
        worker();
        for h in handles {
            if h.join().is_err() {
                broken.store(true, Ordering::Relaxed);
            }
        }
    });
    if mismatch.load(Ordering::Relaxed) {
        Some(false)
    } else if broken.load(Ordering::Relaxed) {
        None
    } else {
        Some(true)
    }
}

/// Decodes one job with `expected` as its history and compares it with `expected`.
fn verify_job(
    plan: &Plan<'_>,
    job: &Job,
    expected: &[u8],
    buffer: &mut Vec<u8>,
    scratch: &mut Scratch,
) -> Option<bool> {
    let want = expected.get(job.out_start..job.out_start.checked_add(job.out_len)?)?;
    let parts = [expected.get(job.session_start..job.out_start)?];
    let history = History::new(&parts);
    buffer.clear();
    // 16 spare bytes let the match copies take their word-sized steps up to the end
    buffer.try_reserve(job.out_len.checked_add(16)?).ok()?;
    buffer.resize(job.out_len + 16, 0);
    decode_job(&plan.blocks, job, Source::Known(&history), buffer, scratch).ok()?;
    Some(buffer.get(..job.out_len)? == want)
}

// ---------------------------------------------------------------------------- streamed pieces

/// The output a streamed decode carries across jobs: the finished jobs of the current session,
/// oldest first, trimmed to the window (their only reader).
#[derive(Default)]
struct Carried {
    parts: Vec<Arc<Vec<u8>>>,
}

impl Carried {
    fn refs(&self) -> Vec<&[u8]> {
        self.parts.iter().map(|p| p.as_slice()).collect()
    }

    fn push(&mut self, slot: Arc<Vec<u8>>, window: usize) {
        self.parts.push(slot);
        let mut len: usize = self.parts.iter().map(|p| p.len()).sum();
        while self.parts.len() > 1 {
            let first = self.parts.first().map_or(0, |p| p.len());
            if len - first < window {
                break;
            }
            len -= first;
            self.parts.remove(0);
        }
    }
}

/// Where a streamed decode hands each job's output: (offset in the whole output, bytes).
pub(crate) type JobSink<'s> = dyn FnMut(usize, &Arc<Vec<u8>>) -> Result<(), Error> + 's;

/// Why a streamed decode stopped.
#[derive(Debug)]
pub(crate) enum PieceError {
    /// The fast path cannot vouch for the result: the caller decodes with the reference decoder.
    Fast,
    /// An error to return as is (the sink's, a limit).
    Stop(Error),
}

/// One piece of an input read as a stream, planned and owning its bytes: each block's range in
/// `buf` with its header, and the jobs over them (output offsets from the start of the input).
pub(crate) struct OwnedPiece {
    buf: Vec<u8>,
    blocks: PieceBlocks,
    jobs: Vec<Job>,
}

/// Each block of a piece: its range in the piece's bytes and its header.
pub(crate) type PieceBlocks = Vec<(std::ops::Range<usize>, crate::dec_lz77::BlockHeader)>;

impl OwnedPiece {
    /// `plan` (of `buf`'s bytes) cut into jobs of about `job_target` bytes; `None` when a block
    /// is not inside `buf`.
    pub(crate) fn layout(buf: &[u8], plan: &Plan<'_>, job_target: usize) -> Option<(PieceBlocks, Vec<Job>)> {
        let base = buf.as_ptr() as usize;
        let blocks = plan
            .blocks
            .iter()
            .map(|b| {
                let start = (b.bytes.as_ptr() as usize).checked_sub(base)?;
                let end = start.checked_add(b.bytes.len())?;
                (end <= buf.len()).then_some((start..end, b.header))
            })
            .collect::<Option<Vec<_>>>()?;
        Some((blocks, plan.jobs(job_target.max(1))))
    }

    pub(crate) fn from_parts(buf: Vec<u8>, parts: (PieceBlocks, Vec<Job>)) -> OwnedPiece {
        OwnedPiece {
            buf,
            blocks: parts.0,
            jobs: parts.1,
        }
    }

    /// Job `j`'s blocks as the decoder takes them, and the job renumbered over them.
    fn job(&self, j: usize) -> Option<(Vec<BlockRef<'_>>, Job)> {
        let job = self.jobs.get(j)?;
        let refs = self
            .blocks
            .get(job.blocks.clone())?
            .iter()
            .map(|(r, h)| {
                Some(BlockRef {
                    bytes: self.buf.get(r.clone())?,
                    header: *h,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let local = Job {
            blocks: 0..refs.len(),
            ..job.clone()
        };
        Some((refs, local))
    }
}

/// What the workers of [`decode_stream`] share: the jobs known so far (piece, job in it), the
/// next to take, how many are finalised, and whether to stop.
struct StreamShared {
    queue: Vec<(Arc<OwnedPiece>, usize)>,
    next: usize,
    resolved: usize,
    ended: bool,
    failed: bool,
}

/// A worker's first pass of one job (index in the queue).
type StreamMsg = (usize, Vec<u8>, Result<FirstPass, Error>);

/// Decodes an input read as a stream, piece after piece (`next_piece()`, `None` at the end), on
/// up to `threads` workers that stay at most `2 * threads` jobs ahead of the calling thread (from
/// one piece into the next: no pause between pieces), which finalises the jobs in order against
/// the window and hands each to `sink(offset, bytes)`. A piece is dropped once its jobs are
/// finalised: the memory held is the pieces in flight, those jobs and the window.
pub(crate) fn decode_stream(
    next_piece: &mut dyn FnMut() -> Result<Option<OwnedPiece>, PieceError>,
    threads: usize,
    sink: &mut JobSink<'_>,
) -> Result<(), PieceError> {
    let threads = thread_count(threads).max(1);
    let lookahead = threads * 2;
    let shared = Mutex::new(StreamShared {
        queue: Vec::new(),
        next: 0,
        resolved: 0,
        ended: false,
        failed: false,
    });
    let wake = Condvar::new();
    let fail = |shared: &Mutex<StreamShared>| {
        if let Ok(mut g) = shared.lock() {
            g.failed = true;
        }
    };
    let worker = |tx: mpsc::Sender<StreamMsg>| {
        let mut scratch = Scratch::new();
        loop {
            let (index, piece, j) = {
                let Ok(mut g) = shared.lock() else { return };
                loop {
                    if g.failed || (g.ended && g.next >= g.queue.len()) {
                        return;
                    }
                    if g.next < g.queue.len() && g.next < g.resolved + lookahead {
                        break;
                    }
                    g = match wake.wait(g) {
                        Ok(g) => g,
                        Err(_) => return,
                    };
                }
                let index = g.next;
                g.next += 1;
                let Some((piece, j)) = g.queue.get(index).cloned() else {
                    return;
                };
                (index, piece, j)
            };
            let first = (|| -> Option<(Vec<u8>, Result<FirstPass, Error>)> {
                let (refs, job) = piece.job(j)?;
                let mut out = zeroed(job.out_len)?;
                let mut pending = Vec::new();
                let cap = pending_cap(&job);
                let result = decode_job(
                    &refs,
                    &job,
                    Source::Deferred {
                        pending: &mut pending,
                        cap,
                    },
                    &mut out,
                    &mut scratch,
                )
                .map(|complete| FirstPass { pending, complete });
                Some((out, result))
            })();
            let Some((out, result)) = first else {
                fail(&shared);
                wake.notify_all();
                return;
            };
            if result.is_err() {
                fail(&shared);
                wake.notify_all();
            }
            if tx.send((index, out, result)).is_err() {
                return;
            }
        }
    };
    std::thread::scope(|scope| {
        let (tx, rx) = mpsc::channel::<StreamMsg>();
        let mut started = 0usize;
        for _ in 0..threads {
            let tx = tx.clone();
            if std::thread::Builder::new()
                .spawn_scoped(scope, move || worker(tx))
                .is_ok()
            {
                started += 1;
            }
        }
        drop(tx);
        let outcome = (|| -> Result<(), PieceError> {
            if started == 0 {
                return Err(PieceError::Fast);
            }
            let empty = Arc::new(OwnedPiece::from_parts(Vec::new(), (Vec::new(), Vec::new())));
            let mut carried = Carried::default();
            let mut scratch = Scratch::new();
            let mut ready: std::collections::HashMap<usize, StreamMsg> = std::collections::HashMap::new();
            let mut index = 0usize;
            let mut ended = false;
            loop {
                // keep the queue `lookahead` jobs past the one to finalise
                loop {
                    let len = shared.lock().map_err(|_| PieceError::Fast)?.queue.len();
                    if ended || len > index + lookahead {
                        break;
                    }
                    match next_piece()? {
                        Some(p) => {
                            let p = Arc::new(p);
                            let mut g = shared.lock().map_err(|_| PieceError::Fast)?;
                            for j in 0..p.jobs.len() {
                                g.queue.push((Arc::clone(&p), j));
                            }
                            drop(g);
                            wake.notify_all();
                        }
                        None => {
                            ended = true;
                            if let Ok(mut g) = shared.lock() {
                                g.ended = true;
                            }
                            wake.notify_all();
                        }
                    }
                }
                let (piece, j) = {
                    let g = shared.lock().map_err(|_| PieceError::Fast)?;
                    match g.queue.get(index) {
                        Some(e) => e.clone(),
                        None => break,
                    }
                };
                let (_, mut out, first) = loop {
                    if let Some(m) = ready.remove(&index) {
                        break m;
                    }
                    let m = rx.recv().map_err(|_| PieceError::Fast)?;
                    ready.insert(m.0, m);
                };
                let (refs, job) = piece.job(j).ok_or(PieceError::Fast)?;
                if job.out_start == job.session_start {
                    carried.parts.clear();
                }
                let first = first.map_err(|_| PieceError::Fast)?;
                {
                    let parts = carried.refs();
                    let h = History::new(&parts);
                    if first.complete {
                        replay(&first.pending, &mut out, &h).map_err(|_| PieceError::Fast)?;
                    } else {
                        drop(first);
                        decode_job(&refs, &job, Source::Known(&h), &mut out, &mut scratch)
                            .map_err(|_| PieceError::Fast)?;
                    }
                }
                let out = Arc::new(out);
                sink(job.out_start, &out).map_err(PieceError::Stop)?;
                carried.push(out, 1usize << job.params.window_size_log2);
                drop(refs);
                if let Ok(mut g) = shared.lock() {
                    g.resolved = index + 1;
                    // the finalised job's place no longer holds its piece
                    if let Some(slot) = g.queue.get_mut(index) {
                        *slot = (Arc::clone(&empty), 0);
                    }
                }
                wake.notify_all();
                index += 1;
            }
            Ok(())
        })();
        if outcome.is_err() {
            fail(&shared);
        }
        if let Ok(mut g) = shared.lock() {
            g.ended = true;
        }
        wake.notify_all();
        outcome
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
pub(crate) mod tests {
    use super::*;
    use crate::datamodel::{
        CHUNK_HEADER_SIZE, CompressionMode, FramingPlan, compress_with_plan, inspect_datamodel,
    };
    use crate::{
        EncoderParams, WINDOW_SIZE_LOG2_MAX, compress, decompress_datamodel_reference, decompress_reference,
    };

    /// `usize::MAX` = the normal cap. Only `deferred_decodes_that_give_up_are_redone` sets it.
    pub(crate) static FORCED_PENDING_CAP: AtomicUsize = AtomicUsize::new(usize::MAX);
    static CAP_LOCK: Mutex<()> = Mutex::new(());

    /// ABF-like bytes: UTF-16 text, small integers, zero runs and noise.
    fn sample(len: usize, seed: u64) -> Vec<u8> {
        let mut x = seed | 1;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let mut out = Vec::with_capacity(len + 4096);
        while out.len() < len {
            match next() % 4 {
                0 => {
                    let text = format!("<Column><Name>Sales {}</Name></Column>", next() % 500);
                    out.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
                }
                1 => (0..64)
                    .for_each(|_| out.extend_from_slice(&(next() & 0x0f0f_0f0f_0f0f_0f0f).to_le_bytes())),
                2 => (0..next() % 300).for_each(|_| out.push(0)),
                _ => (0..8).for_each(|_| out.extend_from_slice(&next().to_le_bytes())),
            }
        }
        out.truncate(len);
        out
    }

    fn units_of(part: &[u8]) -> Vec<(usize, Vec<u8>, usize)> {
        let info = inspect_datamodel(part).unwrap();
        info.chunks
            .iter()
            .map(|c| {
                let start = c.offset + CHUNK_HEADER_SIZE;
                let payload = part[start..start + c.compressed_size as usize].to_vec();
                (c.session, payload, c.uncompressed_size as usize)
            })
            .collect()
    }

    fn fast(part: &[u8], threads: usize) -> Option<Vec<u8>> {
        let owned = units_of(part);
        let units: Vec<Unit<'_>> = owned.iter().map(|(s, p, n)| (*s, &p[..], *n)).collect();
        decode(&units, WINDOW_SIZE_LOG2_MAX, threads)
    }

    fn fast_verify(part: &[u8], expected: &[u8], threads: usize) -> Option<bool> {
        let owned = units_of(part);
        let units: Vec<Unit<'_>> = owned.iter().map(|(s, p, n)| (*s, &p[..], *n)).collect();
        verify(&units, expected, WINDOW_SIZE_LOG2_MAX, threads)
    }

    fn plan_with(chunk_size: usize, mode: CompressionMode) -> FramingPlan {
        FramingPlan {
            chunk_size,
            ..FramingPlan::new(None).with_mode(mode)
        }
    }

    #[test]
    fn the_fast_path_takes_valid_parts_for_any_thread_count() {
        let input = sample(2 << 20, 7);
        for mode in [CompressionMode::Exact, CompressionMode::Fast] {
            for chunk_size in [64 << 10, 512 << 10] {
                let part = compress_with_plan(&input, &plan_with(chunk_size, mode)).unwrap();
                let name = format!("{mode:?}, chunks of {chunk_size}");
                assert!(
                    decompress_datamodel_reference(&part, input.len()).unwrap() == input,
                    "{name}"
                );
                for threads in [1, 2, 3, 8] {
                    let out = fast(&part, threads)
                        .unwrap_or_else(|| panic!("{name}: fell back with {threads} threads"));
                    assert!(out == input, "{name}: {threads} threads");
                    assert_eq!(
                        fast_verify(&part, &input, threads),
                        Some(true),
                        "{name}: verify, {threads} threads"
                    );
                }
            }
        }
    }

    #[test]
    fn verify_reports_a_difference() {
        let input = sample(3 << 20, 9);
        let part = compress_with_plan(&input, &plan_with(256 << 10, CompressionMode::Fast)).unwrap();
        for at in [0, 1 << 20, input.len() - 1] {
            let mut other = input.clone();
            other[at] ^= 1;
            assert_eq!(fast_verify(&part, &other, 4), Some(false), "byte {at}");
            assert!(!crate::verify_datamodel(&part, &other).unwrap(), "byte {at}");
        }
        assert_eq!(fast_verify(&part, &input[..input.len() - 1], 4), Some(false));
        assert!(!crate::verify_datamodel(&part, &input[..input.len() - 1]).is_ok_and(|v| v));
        assert!(crate::verify_datamodel(&part, &input).unwrap());
    }

    #[test]
    fn deferred_decodes_that_give_up_are_redone() {
        let _guard = CAP_LOCK.lock().unwrap();
        let input = sample(3 << 19, 11);
        let part = compress_with_plan(&input, &plan_with(128 << 10, CompressionMode::Exact)).unwrap();
        for cap in [0, 1, 50] {
            FORCED_PENDING_CAP.store(cap, Ordering::Relaxed);
            let out = fast(&part, 4);
            FORCED_PENDING_CAP.store(usize::MAX, Ordering::Relaxed);
            assert!(out.unwrap() == input, "cap {cap}");
        }
    }

    #[test]
    fn the_fast_path_takes_every_stream_shape() {
        let input = sample(600 << 10, 3);
        let zeros = vec![0u8; 3 << 20];
        let shapes = [
            EncoderParams::default(),
            EncoderParams {
                window_size_log2: 16,
                ..EncoderParams::default()
            },
            EncoderParams {
                mtf_entry_count: 0,
                ..EncoderParams::default()
            },
            EncoderParams {
                mtf_entry_count: 2,
                ptr_min_match_length: 3,
                mtf_min_match_length: 3,
                ..EncoderParams::default()
            },
            EncoderParams {
                lookup_depth: 1,
                optimization_level: 1,
                ..EncoderParams::default()
            },
            EncoderParams::power_bi_desktop().0,
        ];
        for (i, params) in shapes.iter().enumerate() {
            for data in [&input[..], &zeros[..], &input[..1], &input[..300]] {
                let stream = compress(data, params).unwrap();
                assert!(
                    decompress_reference(&stream, data.len()).unwrap() == data,
                    "shape {i}"
                );
                for threads in [1, 3] {
                    let out = decode(&[(0, &stream, data.len())], WINDOW_SIZE_LOG2_MAX, threads)
                        .unwrap_or_else(|| panic!("shape {i}, {} bytes: fell back", data.len()));
                    assert!(out == data, "shape {i}, {} bytes", data.len());
                }
            }
        }
    }

    #[test]
    fn the_fast_path_declines_what_it_cannot_vouch_for() {
        let input = sample(100 << 10, 5);
        let stream = compress(&input, &EncoderParams::default()).unwrap();
        let one = |s: &[u8], n: usize, window: u32| decode(&[(0, s, n)], window, 1);
        // wrong expected sizes, a truncated stream, a stream with a trailing byte
        assert!(one(&stream, input.len() + 1, WINDOW_SIZE_LOG2_MAX).is_none());
        assert!(one(&stream, input.len() - 1, WINDOW_SIZE_LOG2_MAX).is_none());
        assert!(one(&stream[..stream.len() - 1], input.len(), WINDOW_SIZE_LOG2_MAX).is_none());
        let mut longer = stream.clone();
        longer.push(0);
        assert!(one(&longer, input.len(), WINDOW_SIZE_LOG2_MAX).is_none());
        // a window above the decoder's maximum
        let params = EncoderParams {
            window_size_log2: 20,
            ..EncoderParams::default()
        };
        let wide = compress(&input, &params).unwrap();
        assert!(one(&wide, input.len(), 17).is_none());
        assert!(one(&wide, input.len(), 20).is_some());
    }
}
