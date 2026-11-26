# XPress9: the compression of the PBIX `DataModel` (Power BI Desktop 2.158.1304.0)

| | |
|---|---|
| Product version | Power BI Desktop Store edition 2.158.1304.0 |
| Date | 2026-10-13 |
| Companion note | `docs/formats/datamodel-abf.md` (public sources, ABF layout, XPress9 framing from pbixray, licence analysis). This note does not repeat it. |

Sources: Microsoft's MIT-licensed XPress9 source in `microsoft/Extensible-Storage-Engine`,
directory `dev/ese/src/_xpress9`, at commit `7030fe7407615160e54d152e4ef704eede2fdd7e` (the
commit each ported file names; the directory's latest change is `4db40470c4bb`), of which `crates/xpress9` is an attributed port;
[MS-XCA]; the CC0 pbixray Kaitai specs; black-box observations of Desktop-saved `DataModel`
parts and of our own parts loaded by Desktop 2.158 (§6). Detailed provenance is kept privately by
the maintainers.

Labels: **Observed** = seen in Desktop-saved files or in Desktop's answers to our files.
**Inferred** = a conclusion from observed facts. **Unknown** = not settled.

---

## 1. Headline: the `DataModel` codec is the ESE XPress9

| Finding | Label |
|---|---|
| The PBIX `DataModel` and the PBIP `.pbi/cache.abf` are compressed with the ESE `_xpress9` format: BIcraft's port of the ESE source decodes every Desktop-saved sample, writes parts Desktop opens, and with Desktop's parameters it reproduces Desktop's streams byte for byte apart from the session signature (section 6). Third-party decoders built on that source (pbixray, ripbi, see `datamodel-abf.md`) decode real PBIX files. | Observed |
| The classic **"Xpress" Plain LZ77** format ([MS-XCA] §2.3/§2.4, also called LZ77 "Win2k3" Xpress or "xpress8") is not on the PBIX `DataModel` path: no Desktop-saved part uses it. | Observed |

Consequence for BIcraft: reading and writing PBIX `DataModel` needs XPress9 (section 3), ported from the ESE MIT source. The Plain LZ77 codec of section 2 is only needed if BIcraft ever reads uncompressed-wrapper ABFs whose inner files are xpress8-compressed (the Excel/legacy case), and it is fully specified by public [MS-XCA].

---

## 2. Plain LZ77 Xpress (not used for `DataModel`)

The format is specified by [MS-XCA] "Plain LZ77": groups of a 32-bit little-endian flag word
(bits consumed from the most significant bit down; 0 = literal byte, 1 = match), matches as a
16-bit token (offset `(token >> 3) + 1`, length from the low 3 bits, extended through a shared
nibble, a byte and a 16-bit value), overlapping copies allowed, no end marker, blocks of at most
65,536 bytes. BIcraft does not implement it.

---

## 3. XPress9 constants (ESE source)

| Item | ESE source value |
|---|---|
| Block header magic | `0x4E86D72A` |
| Encoder context magic (in-memory, not in the stream) | the 4-char constant `exp9` (u32 `0x65787039`) |
| Decoder context magic (in-memory, not in the stream) | the 4-char constant `d9xp` (u32 `0x64397870`) |
| Header CRC table: CRC-32C (Castagnoli, reflected polynomial `0x82F63B78`), 256 × u32 | `Xpress9Misc.c` table |
| Zobrist hashing table used by the encoder, 1,024 × u32 | `Xpress9ZobristTable.h` |
| Status codes, in order: OK, NotEnoughMemory, BadArgument, CorruptedMemory, EncoderNotEmpty, EncoderNotReady, EncoderNotCompressing, EncoderNotFlushing, EncoderCompressing, DecoderCorruptedData, DecoderCorruptedHeader, DecoderWindowTooLarge, DecoderUnknownFormat, DecoderNotDetached, DecoderNotDrained | codes 0–14 in `xpress9.h` |

A faithful port of the ESE source decodes Desktop's `DataModel` and produces streams Desktop accepts (§6).

### 3.1 Block header (ESE layout)

32 bytes, eight LE u32, as listed in `datamodel-abf.md`: magic, original size in bytes, compressed size **in bits**, flags, reserved, session signature, block signature/index, header CRC.

| Detail | Value | Label |
|---|---|---|
| Header CRC | CRC-32C over the first 28 header bytes, with the *reserved* word (word 4) as the running seed. Since reserved = 0 this is plain CRC-32C (init `0xFFFFFFFF`, reflected, final XOR `0xFFFFFFFF`). | ESE source |
| Flags | bits 0–12 Huffman-table length in bits; bits 13–15 window log2 − 16; bits 16–17 MTF entry count / 2; bit 18 min pointer match 3 or 4; bit 19 min MTF match 2 or 3; bits 20–31 must be 0 | ESE source, matches pbixray |
| Window size | The flag field can express 2^16..2^23. The public ESE API limits the window log2 to **16..22** (64 KiB to 4 MiB); internal tables are sized for up to 2^24. This settles the "64 KiB–4 MiB vs 8 MiB" question in `datamodel-abf.md`: a conforming encoder never emits more than 22. | ESE source |

### 3.2 Framing around the blocks

- Two layouts exist: **single-threaded** (banner, then chunks of `u32` uncompressed size, `u32`
  compressed size, payload) and **multithreaded** (banner, then a fixed-size "tasks info" record,
  the 40-byte header of five u64 in `datamodel-abf.md`, then one session per thread group).
- Banner strings (UTF-16LE, needed verbatim by a writer): the two banners documented in
  `datamodel-abf.md`, each **50 characters** long. They share a 30-character common prefix and
  differ by their suffix ("XPress9 compression." vs "multithreaded XPrs9."), which tells the two
  layouts apart. The 102-byte banner field seen in files is the 50 characters plus one UTF-16 NUL
  (Observed on local samples, §6).
- The chunk size is 2 MiB (2,097,152) in every engine- and Desktop-written part seen, and also the
  loader's maximum for a single-threaded chunk (§6.2).

---

## 4. Encoder side (for writing)

| Finding | Label |
|---|---|
| XPress9 encoder parameters live in the ESE encoder parameter struct (window log2, MTF on/off, minimum match lengths, effort). The values Desktop uses can be read back from the flags word of the blocks in any Desktop-saved PBIX (§6). | Observed (method) |
| Any valid ESE encoder output with window log2 ≤ the decoder's configured maximum should be accepted. The decoder rejects larger windows with `DecoderWindowTooLarge`, so a writer should copy the window log2 that Desktop itself writes. | Inferred |
| The decoder checks the session signature (constant within a stream) and the block index sequence (must increase 0, 1, 2…), so a writer must emit one encoder session per stream, and blocks in order. | ESE source; consistent with pbixray and §6.2 |
| The sequential (single-threaded) form is the simpler one to write: banner, then chunks of (u32 uncompressed size, u32 compressed size, payload). Desktop's loader accepts it at every size tried (§6.2). | Observed |
| There is no "stored" XPress9 block type in the ESE format: every block is Huffman + LZ77. A literal-only encoding is still a valid block (all symbols literals, Huffman tables built from literal counts). | ESE source |

---

## 5. Static test vectors

| Vector | Value | Label |
|---|---|---|
| CRC-32C of ASCII `123456789` (check value) | `0xE3069283` | Standard |
| CRC-32C table entries 1, 2, 255 | `0xF26B8303`, `0xE13B70F7`, `0xAD7D5351` | ESE source table (standard CRC-32C) |
| Header CRC of a 28-byte header that is all zero except magic `0x4E86D72A` in word 0 | `0xC9806D35` | Computed |
| Zobrist table row 0 (first four u32) | `0xC70BB2B9, 0xC51E5CA8, 0x60732B88, 0x27B38F05` | ESE source (encoder-only; matters only for byte-identical output) |

---

## 6. Writing the DataModel framing (BIcraft)

`bicraft_xpress9::compress_datamodel(abf, like)` writes the **single-threaded** layout:

| Field | Value written | Source |
|---|---|---|
| Banner, 102 bytes | `like`'s own banner bytes when `like` is single-threaded; otherwise the interoperability identifier `SINGLE_THREADED_BANNER_TEXT` (50 characters, UTF-16LE) followed by one UTF-16 NUL | Observed on local Desktop samples: the field is the 50 characters plus exactly one NUL (answers open question 6) |
| Chunks | `u32` uncompressed size, `u32` compressed size, then the XPress9 blocks of that chunk; every chunk but the last holds `chunk_size` input bytes | `datamodel-abf.md` |
| `chunk_size` | the first chunk's size of a single-threaded `like` with several chunks, the `chunk_uncompressed_size` of a multithreaded `like` (accepted between 64 KiB and 2 MiB), else 2 MiB; never more than 2 MiB | 2 MiB in every engine- and Desktop-written part; the loader refuses larger single-threaded chunks (§6.2) |
| Session | one decoder session for the whole part: one session signature, block indices counting 0, 1, 2… across chunks; every chunk is flushed so it decodes to exactly its envelope size. How the encoder produces it depends on the mode (below) | ESE source; matches the decoder |
| Encoder parameters | `EncoderParams::power_bi_desktop()` (window 2^22, 4 MTF entries, depth 3, lazy, pointer minimum 4, MTF minimum 2, SSE2 insertion), with window, MTF count and minimum match lengths taken from `like`'s first block header | Window, MTF count and minimum lengths: read from the block headers of Desktop-saved parts. The other settings are upstream encoder options; observed: with them, re-encoding a Desktop-saved part reproduces its bytes apart from the session signature and header CRC |
| Session signature | CRC-32C of a 64-bit seed derived from the content (FNV-1a over the length and the first and last 64 KiB), so a given input always gives the same output; Desktop's own signatures are not reproduced, and need not be | Equivalent: any constant signature loads (§6.2) |

### 6.1 Compression modes (`FramingPlan::mode`)

| Mode | How the chunks are encoded | Output |
|---|---|---|
| `Exact` | One encoder session over the whole part; history and MTF state carry across chunks. Single-threaded. | With Desktop's parameters, Desktop's bytes apart from the session signature and header CRC |
| `Balanced` | One encoder per chunk, started fresh (block indices from 0, MTF offsets reset), then *primed*: the 256 KiB of input before the chunk are loaded into its window and hash chains without being encoded, so matches can reach back into earlier chunks. Chunks run on up to 8 threads; afterwards each block's index (header word 6) is rewritten to continue the count and its header CRC recomputed. | A different but valid single session: the decoder has the earlier chunks in its history, and every block header carries its own MTF state. About as small as `Exact` |
| `Fast` (default) | `Balanced` with the lookup depth capped at 1, which also selects upstream's 2^12-entry hash table | 2.5–4% larger than `Exact`; 5–10× faster per thread than `Balanced` |

All modes are deterministic: the output depends only on the input and the plan, not on the
thread count, the scheduling or the platform (checked on Windows and WSL with the same hashes).
Priming is a BIcraft addition to the encoder (`Encoder::prime`); the format is unchanged.

Checks:

- On the local Desktop samples (`tests/testdata.rs`, env-gated): decompressing a sample and
  re-framing it in `Exact` mode with the plan of `inspect_datamodel(part)` gives the same part
  size, banner, chunk envelopes and block headers (sizes, flags, indices); the bytes differ
  only in each block's session signature and header CRC.
- `Exact` reproduces the C oracle's multi-chunk `dm_single` vector (signatures aside).
- Desktop 2.158.1304.0 opened the local sample, with a measure added, written as: `Fast` and
  `Exact` with one 2 MiB chunk; `Exact`, `Balanced`, `Fast` and a greedy depth-1 variant with
  64 KiB chunks (15 chunks, 15 blocks); `Balanced` with 128 and 256 KiB chunks. The new
  measure was present each time. So Desktop accepts our session signatures, chunk sizes other
  than 2 MiB, and blocks encoded independently within one session.
- Single-threaded parts of 113, 578 and 845 MB of ABF load in the engine's `ImageLoad`
  ([`vertipaq-large.md`](vertipaq-large.md)); question 3 is settled for the loader (Desktop itself was checked up to 5.5 MB of ABF, §6.2).

### 6.2 What the loader accepts (G17, 2026-10-15)

Experiment: one PBIX BIcraft built from scratch (a calculated table of 100 000 rows and 39
columns, 5.5 MB of ABF, 3 chunks of 2 MiB by default) re-written once per variant of its
`DataModel` part (cases `variants` / `chunks` / `markers` of the maintainers' development
oracles), each loaded with AMO `ImageLoad` into the engine of Desktop 2.158.1304.0 (Analysis Services 17.0.87.18);
the variants marked (D) were also opened in Desktop 2.158.1304.0 itself, with the same verdict
(the table is in Desktop's model).

| Variant | Verdict |
|---|---|
| default (`Fast`, 2 MiB chunks) (D); `Exact` | loads |
| chunks of 4 KiB (1 344 chunks and blocks) (D); 100 000 bytes (not a power of two); 64 KiB to 2 MiB (§6 checks) | loads |
| chunks of 2 MiB + 1 byte (D), 2.5, 3, 4, 4 MiB + 1, 8 MiB, one 5.5 MiB chunk | **refused** (the load fails while reading the decompressed pages; Desktop opens an empty file) |
| session signature 0 (D), 0xFFFFFFFF, or content-derived | loads |
| session signature changed in the second block | refused (corrupted header) |
| one header CRC wrong | refused (corrupted header) |
| block indices restarting at 0 in every chunk | refused (corrupted header) |
| window 2^16 (D); no MTF entries | loads |
| 16 zero bytes after the last chunk | refused |
| no XPress9 at all: the bare ABF as the `DataModel` part (D) | loads |

So:

- **Chunk size.** 2 MiB is not only Desktop's choice but the loader's maximum for a
  single-threaded chunk; any size from 1 byte (4 KiB tried) to 2 MiB works. Every engine- and
  Desktop-written part seen (single- and multithreaded) uses 2 MiB. Fix: the writer refuses a
  plan above `MAX_CHUNK_SIZE` (2 MiB) and no longer takes a larger size from a sample (the range
  was 64 KiB to 64 MiB; a multithreaded sample with 4 MiB chunks would have produced a file
  Desktop cannot open). Regression test `chunks_never_exceed_two_mebibytes`.
- **Session seed.** Any constant signature is accepted, so the content-derived seed is
  equivalent to the signatures Desktop writes (question 4 settled). The decoder checks constancy,
  the header CRC and the block order, as the ESE source does.
- **Encoder parameters.** Other windows and MTF settings than Desktop's load; BIcraft keeps
  Desktop's (question 1 is settled by the block flags, §6).
- **Framing.** The single-threaded layout is accepted at every size tried, and an uncompressed
  ABF too (`datamodel-abf.md` open question 3a); nothing must follow the last chunk.

---

## 7. Decoding fast (BIcraft)

This section describes BIcraft's own decoder design (`crates/xpress9/src/dec_fast.rs`,
`dec_par.rs`). The format is unchanged.

**What decoding depends on.** A block depends on earlier blocks only through the history. Every
block header carries its own Huffman tables and, when MTF is on, its own MTF state (§3.1), and a
block that decodes correctly never leaves a match unfinished at its end. Block sizes come from
the headers, so the output offset of every block is known before anything is decoded, in both
layouts. In the multithreaded layout, each thread group is its own session, with no shared
history.

**The reference decoder.** `Decoder` (`dec_lz77.rs`) stays a direct port of upstream's streaming
decoder: a history buffer of `1.5 × 2^window` bytes, shifted when full; slices paced by the
compressed bytes at hand; a copy of each slice into the output. It is used as the fallback, and
it is exposed as `decompress_datamodel_reference` and `decompress_reference`.

**The fast decoder.** It decodes each block in one pass, straight into its final place in the
output:

- a 64-bit bit buffer, refilled 7 bytes at a time;
- the upstream root/tail Huffman tables, built by the ported code, with the rare codewords longer
  than 12 bits read through the tail tables as upstream does;
- a 4,096-entry literal table, built from the root table for each block. It holds the literal or
  literals that each value of the next 12 bits starts with: two literals when both codewords fit
  in the 12 bits, which is common for UTF-16 text and small integers;
- match copies in 16- or 8-byte steps when the distance allows, writing up to 15 bytes past the
  match into positions that are not decoded yet; runs with a period of 1 use `fill`, and short
  periods double the copied prefix;
- bounds checks through safe slice access, with no `unsafe`.

**Equivalence with the reference decoder.** Every check of the reference decoder is kept: block
header, session and parameter consistency, table sizes, MTF rules, exact bit and byte counts per
block, and each unit's declared size. One check is stricter: a match may reach back at most
`min(session position, 2^window)` bytes. Upstream accepts at least that much, because its buffer
always holds the last `2^window` bytes or everything since the session began. Upstream's encoder,
and so ours (§6), never reaches further, because its own buffer is `2^window` bytes. Hence, when
the fast decoder succeeds, the reference decoder succeeds with the same bytes. On *any* failure,
`decompress`/`decompress_datamodel` rerun the reference decoder, so errors, and the theoretical
stream that only the reference accepts, are reported exactly as before. The fuzz smoke tests
assert this equality on every mutated input.

**Parallel decoding** (`dec_par.rs`). Used for the read path, on one thread per core, at most
`MAX_DECODE_THREADS` = 32 (8 until 2026-10-25: a 109 MB backup of 53 blocks decodes in 37–45 ms
on 20 threads against 77–83 ms on 8, 345–357 ms on one; the calling thread's in-order replay
of recorded matches is 6–7 ms of it, the rest is the workers').

- The output is one zeroed allocation of the declared size, taken from the allocator's zeroed
  allocation after a fallible reservation of that size: fresh pages are not written first, the
  workers fault them in as they decode (zeroing 109 MB on the calling thread took ~10 ms).

- Every block header is parsed and checked first, sequentially. Then consecutive blocks of a
  session are grouped into jobs: about four per thread, 256 KiB to 4 MiB each. With Desktop's
  2 MiB chunks of one block each, a job is one block.
- Workers decode jobs out of order. A job that does not start its session is decoded with its
  history unknown ("deferred"). It writes literals, plus the matches whose source lies inside the
  job and is already known. A match that reads the history, or bytes that are still unknown, is
  only recorded: position, distance and length, 12 bytes. The unknown ranges are kept as sorted
  intervals.
- The calling thread finalises jobs in order. Once the jobs before a job are final, it replays the
  job's recorded matches in order with exact copies. This gives the sequential result: literals
  never move, and every replayed match reads only bytes that are final by then.
- A deferred decode that would record more than `len / 16` matches gives up. The calling thread
  then decodes that job again, with its history known.
- Workers stay at most `2 × threads` jobs ahead of the finalised ones. This bounds the memory
  held by recorded matches to about `2 × threads × 0.75 × job size`.
- The output is allocated once, after the header pass: the declared size, which is already
  checked against the caller's limit. It does not depend on the thread count or the scheduling.

**Checking a written part** (`verify_datamodel(part, expected)`). It answers
`decompress_datamodel(part, expected.len())? == expected` without building the output:

- Each job is decoded on its own, in parallel, into a per-thread buffer. Its history is
  `expected` itself, and the result is compared with its slice of `expected`.
- If every job matches, the whole part decodes to `expected`, by induction over the jobs: each job
  sees exactly the history the real decode would give it.
- Anything other than a clean match (a mismatch or a fast-path failure) is settled by the
  reference decoder, so the answer, and the error for an undecodable part, never depend on the
  path taken.

`bicraft_pbix::write_data_model` and the structural writer use it after compressing. When the
re-packed ABF is byte-identical to the one read, `write_data_model` returns the original part
unchanged, and nothing is re-compressed.

Checks:

- Unit tests (`dec_par.rs`) assert that the fast path, not the fallback, takes these inputs:
  `Exact` and `Fast` parts with 64 and 512 KiB chunks, decoded on 1, 2, 3 and 8 threads; every
  encoder parameter shape; and the forced give-up path. They also assert that the fast path
  declines wrong sizes, truncation and trailing bytes.
- On the local samples (`tests/testdata.rs`, env-gated), the fast path takes Desktop's streams,
  and the output is byte-identical to the reference decoder's on 1 and 3 threads.
- A one-off check decoded every local sample with both decoders and found byte-identical
  results: all PBIX `DataModel` parts and `cache.abf` files, a 113 MB single-threaded part, and
  two multithreaded parts of 578 MB and 845 MB with 10 sessions each, compared session by
  session.

---

## 8. Streaming (BIcraft)

`bicraft_xpress9::stream_datamodel` decodes a `DataModel` part read from any `Read`, front to
back, and hands each decoded chunk to a sink with its offset in the backup; the sink can stop the
decode. Memory is one chunk's payload and output (each envelope size capped at 256 MiB,
`MAX_STREAM_CHUNK`) plus the decoder's own history buffer (the window, at most 4 MiB, is kept by
`Decoder`, not by the caller's output), so a part of any size decodes in constant memory. The
three layouts are handled as in §6: a bare ABF is passed through, a single-threaded part is one
session, a multithreaded part restarts the session for each thread group. Used for backups too
large to decompress into memory (`pbix-package.md` §11, "Packages over 4 GiB").

## Open questions

1. ~~The exact XPress9 encoder parameters Desktop uses~~: read from the block flags (window 2^22, 4 MTF entries, minimum matches 4 and 2, §6; the effort is not recorded in a stream). Other windows and MTF settings load too (§6.2).
2. Whether the chunk size is a fixed engine constant (it is 2 MiB in every part seen, and the loader's single-threaded maximum, §6.2), and whether the multithreaded writer's thread and chunk counts follow a fixed rule.
3. ~~Whether Desktop accepts a single-threaded XPress9 stream for any model size~~: settled for the loader (§6.2): chunks of at most 2 MiB, any count. Observed with a local engine instance (AMO `ImageLoad`, [`vertipaq-large.md`](vertipaq-large.md)): images re-compressed by `compress_datamodel` (single-threaded banner and chunks) load, 43 MB, 380 MB and 415 MB for 113, 578 and 845 MB of ABF; `ImageSave` itself wrote the single-threaded form at 113 MB of ABF and the multithreaded form at 578 MB and 845 MB.
4. ~~Whether the session signature has to be anything in particular~~: no; 0, 0xFFFFFFFF and content-derived signatures load, a change within the session does not (§6.2).
5. Which engine paths still use Plain LZ77 (legacy ABF inner files, or something else). Not needed for `DataModel`.
6. ~~Whether the 102-byte banner field ends with a NUL or with another padding unit after the 50 characters.~~ Settled on local samples: 50 characters, then one UTF-16 NUL (section 6).
7. Chunk-loop details (size limits, error handling, multithreaded order) rest on `datamodel-abf.md`, the ESE source and the loader experiments of §6.2.

## Suggested verification

1. Save a small model from Desktop, extract `/DataModel`, and decode it with a port of the ESE decoder (max window log2 22). Compare the result with an ABF produced by an Analysis Services backup with `ApplyCompression=false` of the same database (Tabular Editor / DAX Studio against Desktop's local instance), byte for byte after the banner.
2. Dump every block header of a Desktop-saved stream: check magic, the CRC-32C rule, block index order, session-signature constancy, and record the flags (open question 1).
3. Round trip: re-encode the decoded ABF with the ported encoder using the observed flags, write it in a PBIX, and open it in Desktop.
4. Do the same with a large model (> 2 MiB × threads) to get a multithreaded stream and confirm the 40-byte header semantics.
5. Optional: run the CRC-32C and Zobrist tables of the Rust port against the constants in section 5 as unit tests (tables are generated, not copied).
