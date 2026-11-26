# bicraft-xpress9

Pure-Rust XPress9 codec (decoder and encoder) and the PBIX `DataModel` framing around it.
Layer L0 of BIcraft; no internal dependencies, no `unsafe`, no C.

## Provenance

The codec is a port of Microsoft's MIT-licensed XPress9 C sources:
`microsoft/Extensible-Storage-Engine`, `dev/ese/src/_xpress9`, commit
`7030fe7407615160e54d152e4ef704eede2fdd7e`. Every ported file names its upstream file and
commit in a header. The license is in [`LICENSE-MIT-microsoft-xpress9`](LICENSE-MIT-microsoft-xpress9)
and in the repository's `THIRD_PARTY_NOTICES.md`.

| Rust module | Upstream |
|---|---|
| `misc.rs` | `Xpress9Misc.c`, constants and bit-I/O macros of `Xpress9Internal.h` |
| `dec_huffman.rs` | `Xpress9DecHuffman.c`, `HUFFMAN_DECODE_SYMBOL` |
| `dec_lz77.rs` | `Xpress9DecLz77.c`, `Xpress9Lz77Dec.i` |
| `enc_huffman.rs` | `Xpress9EncHuffman.c` |
| `enc_lz77.rs` | `Xpress9EncLz77.c`, `Xpress9Lz77EncPass1.i`, `Xpress9Lz77EncPass2.i`, `Xpress9Lz77EncInsert.i`, `Xpress9Lookup.i` (+ `.i2`, `.i3`) |
| `zobrist.rs` | `Xpress9ZobristTable.h` (data) |
| `datamodel.rs` | original; framing from the CC0 `abf_xpress9.ksy` of pbixray |
| `dec_fast.rs` | the decode loop of `Xpress9Lz77Dec.i` re-implemented for whole blocks (fast path) |
| `dec_par.rs` | original; runs `dec_fast.rs` on several threads, and the parallel `verify_datamodel` |

Translation notes (the bit stream and all validity checks are unchanged):

- The preprocessor instantiations (10 decoder loops, 30 encoder pass-1 variants) are single
  functions driven by the session parameters.
- Upstream's push/pull streaming API (`Attach` / `Fetch` with an 8 KiB scratch area) is folded
  into whole-block calls: `Decoder::decode_block` and `Encoder::compress`. History, MTF state,
  block indices and session signatures still carry across calls within a session.
- `RETAIL_ASSERT` failures become `Error::Internal`; every buffer access is bounds-checked.
- Hardening beyond upstream: a block whose window exceeds the decoder maximum is rejected
  (upstream would underflow), a failed decoder session must be restarted, and output is capped by
  the caller.
- The session signature is `Xpress9Crc32(seed)` with a caller-supplied seed instead of
  `__rdtsc()`, so output is reproducible.
- The SSE2 hash insertion (`Xpress9Flag_UseSSE2`, minimum pointer length 4) is reproduced with
  scalar code (`FLAG_USE_SSE2`). It changes which matches are found, not the format.

## API

```rust
use bicraft_xpress9::{compress, decompress, decompress_datamodel, inspect_datamodel, verify_datamodel, EncoderParams};

let packed = compress(b"hello hello hello", &EncoderParams::default())?;
let plain = decompress(&packed, 17)?;                  // exact expected length, also the cap
let abf = decompress_datamodel(&datamodel_part, 4 << 30)?; // PBIX DataModel or PBIP cache.abf -> ABF
let info = inspect_datamodel(&datamodel_part)?;        // framing summary, no decompression
let same = verify_datamodel(&datamodel_part, &abf)?;   // decodes to exactly these bytes?
```

`Decoder` and `Encoder` expose the session API. `EncoderParams::power_bi_desktop()` returns the
parameters and flags that reproduce Power BI Desktop's output (see below).

## The DataModel framing

A 102-byte UTF-16LE banner (character 30 onwards says `multithreaded` for the multithreaded
form), then either a flat list of chunks (single-threaded, one XPress9 session) or a 40-byte
thread-distribution header (five u64) followed by per-thread chunk groups, each its own session.
A chunk is `u32 uncompressed size, u32 compressed size`, then whole XPress9 blocks. A part that is
already an ABF is passed through.

Observed on a local sample saved by Power BI Desktop 2.158.1304.0 (a PBIX `DataModel` and the
matching PBIP `cache.abf`, about 0.9 MiB of ABF each; the sample is held locally and is not in
the repository): both are single-threaded, one chunk holding one block, window 2^22, 4 MTF entries,
minimum lengths 4/2. Re-encoding the decompressed ABF with `EncoderParams::power_bi_desktop()`
(lookup depth 3, lazy evaluation, SSE2 hashing: upstream encoder options) reproduces Desktop's
compressed bytes exactly, apart from the session signature and the header CRC.

## Tests and test vectors

`tests/vectors/*.x9` were produced by the upstream C code, compiled with gcc in WSL as an
oracle. The inputs are not stored: `tests/common/mod.rs`
regenerates them from a seeded splitmix64 PRNG, exactly like the C driver. `manifest.txt` lists
each vector's generator, size, seed and encoder parameters. The set covers sizes 0/1/3/4/7,
1 MiB of zeros, incompressible data, text, 3 MiB of structured records, sizes around the 64 KiB
window, multi-block streams with window shifts, every MTF / minimum-length / depth combination,
lazy evaluation, a shrunken window (`max_stream_length`), a chunked session, and synthetic
single- and multithreaded `DataModel` parts.

The tests check that:

- the Rust decoder reproduces every input;
- the Rust encoder emits **the same bytes** as the C encoder for every vector;
- encoder output round-trips for many parameter sets (and the C decoder accepted Rust-encoded
  streams in a one-off check: `BICRAFT_XPRESS9_EXPORT=<dir>` writes them);
- truncated, bit-flipped and random inputs give `Err` without panicking
  (`BICRAFT_FUZZ_SCALE=<n>` multiplies the iterations), and the fast decoder returns exactly
  what the reference decoder returns (`tests/fuzz_smoke.rs`);
- the fast decoder, not the fallback, takes valid streams of every shape on 1–8 threads
  (unit tests in `src/dec_par.rs`);
- with `BICRAFT_TESTDATA=<dir>`, real Desktop samples (kept outside the repo) decode to ABFs,
  on the fast path and byte-identical to the reference decoder (`tests/testdata.rs`; skipped
  when unset).

Regenerating the vectors (development machine, WSL with gcc):

```bash
# fetch dev/ese/src/_xpress9 at the commit above into $SRC (outside the repository), then:
gcc -O2 -fno-strict-aliasing -w -include oracle/shim.h -I"$SRC" -o x9driver oracle/driver.c \
    "$SRC"/Xpress9DecLz77.c "$SRC"/Xpress9DecHuffman.c "$SRC"/Xpress9EncLz77.c \
    "$SRC"/Xpress9EncHuffman.c "$SRC"/Xpress9Misc.c
./x9driver gen tests/vectors        # writes *.x9 and manifest.txt, after a C round trip
./x9driver dec <file.x9> <length>   # decode any stream with the C decoder
```

The source directory must be case-insensitive (the sources include `Xpress9.h` while the file
is `xpress9.h`); a Windows directory mounted under `/mnt/c` works.

The banners of `dm_single.x9` and `dm_multi.x9` were rewritten in place on 2026-10-28 when the
project name in `driver.c` changed (102 bytes, outside every checksum); the rest of each file is
the driver's output.

Throughput (release, one core, 4 MiB inputs): decoding 300–800 MB/s, encoding 55–120 MB/s
(`cargo test --release -p bicraft-xpress9 --test vectors -- --ignored bench --nocapture`).

## Decoding

`decompress`, `decompress_datamodel` and `verify_datamodel` run a fast decoder
(`dec_fast.rs`): whole blocks decoded in place, a 64-bit bit buffer, a two-literal lookup table
and word-sized match copies. Blocks are decoded on up to `MAX_DECODE_THREADS` (32) threads, at most one per core.
Blocks whose history is not decoded yet record the matches that need it, and those matches are
replayed in order afterwards. Its checks are those of the ported `Decoder`, plus a stricter
match reach that upstream's encoder never exceeds. On any failure the ported decoder
(`decompress_reference`, `decompress_datamodel_reference`) runs instead, so bytes and errors are
always the reference decoder's. The fuzz smoke tests assert this on every input. Measured: 1.6–2×
on one thread, 4–6× with threads on multi-chunk parts; the parallel `verify_datamodel` is 8–14×
faster than decompress-and-compare. See `docs/formats/xpress9.md` §7 and
`docs/benchmarks.md`.
On larger inputs the exact encoder drops to 3–10 MB/s: its match-finder tables leave the
caches.

## Compression modes

`compress_datamodel` uses `CompressionMode::Fast`: every chunk gets its own encoder, primed
with the 256 KiB of input before it (`Encoder::prime`, a BIcraft addition), with a lookup
depth of 1, on up to 8 threads; the blocks are then renumbered into one session. 200–300 MB/s
on 16–192 MB inputs, parts 2.5–4% larger than Desktop's. `CompressionMode::Balanced` keeps
Desktop's match search (same size as `Exact`, 25–50 MB/s); `CompressionMode::Exact` is the
single-encoder path that reproduces Desktop's bytes. Choose one with `compress_with_plan` and
`FramingPlan::with_mode`; `FramingPlan::threads` sets the worker count (the output never
depends on it). See `docs/formats/xpress9.md` §6.1.
