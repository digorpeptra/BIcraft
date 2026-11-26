/*
 * Test-vector generator / oracle driver for bicraft-xpress9 (not shipped; dev only).
 * bicraft's own code (MIT OR Apache-2.0). It is compiled together with Microsoft's XPress9
 * C sources, which are NOT part of this repository; see ../README.md for the build command.
 *   driver gen <outdir>          compress the vector set, verify with the C decoder, write
 *                                <outdir>/<name>.x9 and <outdir>/manifest.txt
 *   driver dec <file> <len>      decode an XPress9 stream with the C decoder, print FNV-1a
 *   driver dm <file> <len>       decode a synthetic DataModel part (single/multi) and print FNV-1a
 */
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include "xpress9.h"

unsigned long long g_fake_tsc = 0x0123456789abcdefULL;

static void *XPRESS9_CALL alloc_cb(void *ctx, int size) { (void)ctx; return calloc(1, (size_t)size); }
static void XPRESS9_CALL free_cb(void *ctx, void *p) { (void)ctx; free(p); }

/* ------------------------------------------------------------------ deterministic inputs */
static uint64_t sm_state;
static uint64_t next_u64(void)
{
    uint64_t z = (sm_state += 0x9E3779B97F4A7C15ULL);
    z = (z ^ (z >> 30)) * 0xBF58476D1CE4E5B9ULL;
    z = (z ^ (z >> 27)) * 0x94D049BB133111EBULL;
    return z ^ (z >> 31);
}

static const char *WORDS[12] = {"alpha", "beta", "gamma", "delta", "epsilon", "zeta",
                                "eta", "theta", "iota", "kappa", "lambda", "mu"};

typedef struct { uint8_t *p; size_t len, cap; } buf_t;
static void put(buf_t *b, uint8_t v) { if (b->len < b->cap) b->p[b->len++] = v; }

static void gen_text(buf_t *b, size_t size)
{
    while (b->len < size) {
        const char *w = WORDS[next_u64() % 12];
        for (; *w && b->len < size; ++w) put(b, (uint8_t)*w);
        put(b, (next_u64() % 8 == 0) ? '\n' : ' ');
    }
}

static uint8_t *generate(const char *gen, size_t size, uint64_t seed)
{
    buf_t b; b.p = calloc(1, size + 1); b.len = 0; b.cap = size;
    sm_state = seed;
    if (!strcmp(gen, "zeros")) { b.len = size; }
    else if (!strcmp(gen, "random")) { while (b.len < size) put(&b, (uint8_t)(next_u64() >> 56)); }
    else if (!strcmp(gen, "text")) { gen_text(&b, size); }
    else if (!strcmp(gen, "struct")) {
        uint32_t id = 0;
        while (b.len < size) {
            uint8_t rec[16]; uint32_t cat = (uint32_t)(next_u64() % 2), amount = (uint32_t)(next_u64() % 16);
            memset(rec, 0, 16);
            memcpy(rec, &id, 4); rec[4] = (uint8_t)cat; memcpy(rec + 8, &amount, 4);
            for (int i = 0; i < 16; ++i) put(&b, rec[i]);
            id = (id + 1) & 255;
        }
    } else if (!strcmp(gen, "mixed")) {
        uint64_t last_off = 1;
        while (b.len < size) {
            uint64_t r = next_u64() % 4;
            if (r == 0) { uint64_t n = 1 + next_u64() % 64; for (uint64_t i = 0; i < n; ++i) put(&b, (uint8_t)(next_u64() >> 56)); }
            else if (r == 1) { size_t lim = b.len + 16 + next_u64() % 497; if (lim > size) lim = size; gen_text(&b, lim); }
            else if (r == 2) {
                if (b.len == 0) continue;
                uint64_t off = (next_u64() % 2) ? last_off : 1 + next_u64() % (b.len < 70000 ? b.len : 70000);
                if (off > b.len) off = b.len;
                last_off = off;
                uint64_t n = 3 + next_u64() % 300;
                for (uint64_t i = 0; i < n; ++i) put(&b, b.p[b.len - off]);
            } else { uint8_t v = (uint8_t)(next_u64() >> 56); uint64_t n = 1 + next_u64() % 2000; for (uint64_t i = 0; i < n; ++i) put(&b, v); }
        }
    } else { fprintf(stderr, "unknown generator %s\n", gen); exit(2); }
    return b.p;
}

/* -------------------------------------------------------------------------- C codec use */
typedef struct {
    const char *name, *kind, *gen; size_t size; uint64_t seed;
    unsigned maxwin, window, mtf, depth, opt, ptrmin, mtfmin, maxstream; size_t chunk;
    unsigned pt, pp, mt, mp;
} vec_t;

static void die(const char *what, XPRESS9_STATUS *st) { fprintf(stderr, "%s failed: status %u\n", what, st->m_uStatus); exit(1); }

static XPRESS9_ENCODER new_session(const vec_t *v, XPRESS9_ENCODER enc)
{
    XPRESS9_STATUS st;
    XPRESS9_ENCODER_PARAMS p; memset(&p, 0, sizeof p);
    p.m_cbSize = sizeof p; p.m_uMaxStreamLength = v->maxstream; p.m_uMtfEntryCount = v->mtf;
    p.m_uLookupDepth = v->depth; p.m_uWindowSizeLog2 = v->window; p.m_uOptimizationLevel = v->opt;
    p.m_uPtrMinMatchLength = v->ptrmin; p.m_uMtfMinMatchLength = v->mtfmin;
    Xpress9EncoderStartSession(&st, enc, &p, 1);
    if (st.m_uStatus) die("StartSession", &st);
    return enc;
}

/* attach one buffer with flush, return compressed bytes (appended to out) */
static size_t compress_attach(XPRESS9_ENCODER enc, const uint8_t *data, size_t n, uint8_t *out, size_t cap)
{
    XPRESS9_STATUS st; size_t total = 0;
    Xpress9EncoderAttach(&st, enc, n ? data : NULL, (unsigned)n, 1);
    if (st.m_uStatus) die("Attach", &st);
    for (;;) {
        unsigned promised = Xpress9EncoderCompress(&st, enc, NULL, NULL);
        if (st.m_uStatus) die("Compress", &st);
        if (promised == 0) break;
        unsigned remaining;
        do {
            unsigned got = 0;
            remaining = Xpress9EncoderFetchCompressedData(&st, enc, out + total, (unsigned)(cap - total), &got);
            if (st.m_uStatus) die("Fetch", &st);
            total += got;
        } while (remaining != 0);
    }
    Xpress9EncoderDetach(&st, enc, n ? data : NULL, (unsigned)n);
    if (st.m_uStatus) die("Detach", &st);
    return total;
}

/* decode one attached buffer into out; returns bytes produced or (size_t)-1 */
static size_t decode_attach(XPRESS9_DECODER dec, const uint8_t *comp, size_t n, uint8_t *out, size_t want)
{
    XPRESS9_STATUS st; size_t total = 0; int guard = 0;
    if (n == 0) return 0;
    Xpress9DecoderAttach(&st, dec, comp, (unsigned)n);
    if (st.m_uStatus) return (size_t)-1;
    while (total < want && guard++ < 1000000) {
        unsigned written = 0, needed = 0;
        Xpress9DecoderFetchDecompressedData(&st, dec, out + total, (unsigned)(want - total), &written, &needed);
        if (st.m_uStatus) { fprintf(stderr, "decoder status %u\n", st.m_uStatus); return (size_t)-1; }
        total += written;
        if (written == 0 && needed != 0) break;
    }
    Xpress9DecoderDetach(&st, dec, comp, (unsigned)n);
    if (st.m_uStatus) { fprintf(stderr, "detach status %u\n", st.m_uStatus); return (size_t)-1; }
    return total;
}

static uint64_t fnv1a(const uint8_t *p, size_t n)
{
    uint64_t h = 0xcbf29ce484222325ULL;
    for (size_t i = 0; i < n; ++i) { h ^= p[i]; h *= 0x100000001b3ULL; }
    return h;
}

static void put_u32(uint8_t *p, uint32_t v) { memcpy(p, &v, 4); }
static void put_u64(uint8_t *p, uint64_t v) { memcpy(p, &v, 8); }

static size_t banner(uint8_t *out, int multi)
{
    const char *t = multi ? "BIcraft synthetic test files: multithreaded XPress9"
                          : "BIcraft synthetic test files: single-threaded XPress9";
    memset(out, 0, 102);
    for (size_t i = 0; t[i] && i < 50; ++i) out[2 * i] = (uint8_t)t[i];
    return 102;
}

/* compress a vector; returns malloc'd buffer */
static uint8_t *build(const vec_t *v, const uint8_t *in, size_t *outlen)
{
    size_t cap = v->size * 2 + 1024 * 1024;
    uint8_t *out = calloc(1, cap); size_t total = 0;
    XPRESS9_STATUS st;
    XPRESS9_ENCODER enc = Xpress9EncoderCreate(&st, NULL, alloc_cb, v->maxwin, 0);
    if (!enc) die("EncoderCreate", &st);
    if (!strcmp(v->kind, "stream")) {
        new_session(v, enc);
        if (v->chunk == 0) total = compress_attach(enc, in, v->size, out, cap);
        else for (size_t off = 0; off < v->size; off += v->chunk) {
            size_t n = v->size - off < v->chunk ? v->size - off : v->chunk;
            total += compress_attach(enc, in + off, n, out + total, cap - total);
        }
    } else {
        int multi = !strcmp(v->kind, "dm_multi");
        total = banner(out, multi);
        size_t off = 0;
        if (!multi) {
            new_session(v, enc);
            for (; off < v->size; off += v->chunk) {
                size_t n = v->size - off < v->chunk ? v->size - off : v->chunk;
                uint8_t *env = out + total; total += 8;
                size_t c = compress_attach(enc, in + off, n, out + total, cap - total);
                put_u32(env, (uint32_t)n); put_u32(env + 4, (uint32_t)c); total += c;
            }
        } else {
            put_u64(out + total, v->mp); put_u64(out + total + 8, v->pp); put_u64(out + total + 16, v->pt);
            put_u64(out + total + 24, v->mt); put_u64(out + total + 32, v->chunk); total += 40;
            unsigned groups[2][2] = {{v->pt, v->pp}, {v->mt, v->mp}};
            for (int g = 0; g < 2; ++g)
                for (unsigned t = 0; t < groups[g][0]; ++t) {
                    new_session(v, enc);
                    for (unsigned c = 0; c < groups[g][1]; ++c) {
                        size_t n = v->size - off < v->chunk ? v->size - off : v->chunk;
                        uint8_t *env = out + total; total += 8;
                        size_t cl = compress_attach(enc, in + off, n, out + total, cap - total);
                        put_u32(env, (uint32_t)n); put_u32(env + 4, (uint32_t)cl); total += cl; off += n;
                    }
                }
            if (off != v->size) { fprintf(stderr, "%s: chunk layout covers %zu of %zu bytes\n", v->name, off, v->size); exit(1); }
        }
    }
    Xpress9EncoderDestroy(&st, enc, NULL, free_cb);
    *outlen = total;
    return out;
}

/* decode a stream or a synthetic DataModel part; returns produced length or -1 */
static size_t decode_any(const uint8_t *comp, size_t n, uint8_t *out, size_t want, int dm)
{
    XPRESS9_STATUS st;
    XPRESS9_DECODER dec = Xpress9DecoderCreate(&st, NULL, alloc_cb, XPRESS9_WINDOW_SIZE_LOG2_MAX, 0);
    if (!dec) die("DecoderCreate", &st);
    Xpress9DecoderStartSession(&st, dec, 1);
    size_t total = 0;
    if (!dm) total = decode_attach(dec, comp, n, out, want);
    else {
        int multi = comp[60] == 'm';  /* character 30 of the banner */
        size_t at = 102;
        if (!multi) {
            while (at + 8 <= n) {
                uint32_t u, c; memcpy(&u, comp + at, 4); memcpy(&c, comp + at + 4, 4); at += 8;
                size_t r = decode_attach(dec, comp + at, c, out + total, u);
                if (r != u) return (size_t)-1;
                total += r; at += c;
            }
        } else {
            uint64_t h[5]; memcpy(h, comp + at, 40); at += 40;
            uint64_t groups[2][2] = {{h[2], h[1]}, {h[3], h[0]}};
            for (int g = 0; g < 2; ++g)
                for (uint64_t t = 0; t < groups[g][0]; ++t) {
                    Xpress9DecoderStartSession(&st, dec, 1);
                    for (uint64_t k = 0; k < groups[g][1]; ++k) {
                        uint32_t u, c; memcpy(&u, comp + at, 4); memcpy(&c, comp + at + 4, 4); at += 8;
                        size_t r = decode_attach(dec, comp + at, c, out + total, u);
                        if (r != u) return (size_t)-1;
                        total += r; at += c;
                    }
                }
        }
    }
    Xpress9DecoderDestroy(&st, dec, NULL, free_cb);
    return total;
}

static uint8_t *read_file(const char *path, size_t *len)
{
    FILE *f = fopen(path, "rb"); if (!f) { perror(path); exit(1); }
    fseek(f, 0, SEEK_END); long n = ftell(f); fseek(f, 0, SEEK_SET);
    uint8_t *p = malloc((size_t)n + 1); if (fread(p, 1, (size_t)n, f) != (size_t)n) exit(1); fclose(f);
    *len = (size_t)n; return p;
}

/* Default params: the xpress9-python Compress settings. */
#define DEF 22, 22, 4, 9, 0, 4, 2, 0
static const vec_t VECTORS[] = {
    /* name                     kind        gen       size        seed  maxwin..maxstream                chunk  pt pp mt mp */
    {"empty",                   "stream",   "zeros",  0,          1,    DEF,                             0},
    {"one_byte",                "stream",   "text",   1,          2,    DEF,                             0},
    {"three_bytes",             "stream",   "text",   3,          3,    DEF,                             0},
    {"four_bytes",              "stream",   "text",   4,          4,    DEF,                             0},
    {"seven_bytes",             "stream",   "text",   7,          5,    DEF,                             0},
    {"text_100",                "stream",   "text",   100,        6,    DEF,                             0},
    {"zeros_1m",                "stream",   "zeros",  1 << 20,    7,    DEF,                             0},
    {"random_16k",              "stream",   "random", 16384,      8,    DEF,                             0},
    {"text_256k",               "stream",   "text",   262144,     9,    DEF,                             0},
    {"struct_3m",               "stream",   "struct", 3 << 20,    10,   DEF,                             0},
    {"mixed_w16_65535",         "stream",   "mixed",  65535,      11,   16, 16, 4, 9, 0, 4, 2, 0,        0},
    {"mixed_w16_65536",         "stream",   "mixed",  65536,      12,   16, 16, 4, 9, 0, 4, 2, 0,        0},
    {"mixed_w16_65537",         "stream",   "mixed",  65537,      13,   16, 16, 4, 9, 0, 4, 2, 0,        0},
    {"text_w16_multiblock",     "stream",   "text",   400000,     14,   16, 16, 4, 9, 0, 4, 2, 0,        0},
    {"random_w16_multiblock",   "stream",   "random", 70000,      15,   16, 16, 4, 9, 0, 4, 2, 0,        0},
    {"mixed_w17_multiblock",    "stream",   "mixed",  300000,     16,   17, 17, 4, 9, 0, 4, 2, 0,        0},
    {"p_mtf0_ptr3_d0",          "stream",   "mixed",  40000,      17,   22, 22, 0, 0, 0, 3, 2, 0,        0},
    {"p_mtf0_ptr4_d1",          "stream",   "mixed",  40000,      18,   22, 22, 0, 1, 0, 4, 3, 0,        0},
    {"p_mtf2_ptr3_mtf3_d2",     "stream",   "mixed",  40000,      19,   22, 22, 2, 2, 0, 3, 3, 0,        0},
    {"p_mtf2_ptr4_mtf2_d0",     "stream",   "mixed",  40000,      20,   22, 22, 2, 0, 0, 4, 2, 0,        0},
    {"p_mtf4_ptr3_mtf2_d5",     "stream",   "mixed",  40000,      21,   22, 20, 4, 5, 0, 3, 2, 0,        0},
    {"p_mtf4_ptr4_mtf3_d1",     "stream",   "mixed",  40000,      22,   22, 18, 4, 1, 0, 4, 3, 0,        0},
    {"p_mtf4_ptr3_mtf3_d0",     "stream",   "mixed",  40000,      23,   22, 22, 4, 0, 0, 3, 3, 0,        0},
    {"p_lazy_ptr4_d9",          "stream",   "mixed",  60000,      24,   22, 22, 4, 9, 1, 4, 2, 0,        0},
    {"p_lazy_ptr3_d4",          "stream",   "mixed",  60000,      25,   22, 22, 4, 4, 1, 3, 2, 0,        0},
    {"p_lazy_ptr4_d20",         "stream",   "text",   60000,      26,   22, 22, 4, 20, 1, 4, 2, 0,       0},
    {"p_maxstream_3000",        "stream",   "mixed",  3000,       27,   22, 22, 4, 9, 0, 4, 2, 3000,     0},
    {"chunked_w16_session",     "stream",   "mixed",  200000,     28,   16, 16, 4, 9, 0, 4, 2, 0,        40000},
    {"dm_single",               "dm_single","mixed",  150000,     29,   22, 22, 4, 9, 0, 4, 2, 0,        32768},
    {"dm_multi",                "dm_multi", "mixed",  190000,     30,   22, 22, 4, 9, 0, 4, 2, 0,        32768, 1, 2, 2, 2},
};

int main(int argc, char **argv)
{
    if (argc >= 3 && !strcmp(argv[1], "gen")) {
        char path[4096];
        snprintf(path, sizeof path, "%s/manifest.txt", argv[2]);
        FILE *man = fopen(path, "w"); if (!man) { perror(path); return 1; }
        fprintf(man, "# name kind gen size seed maxwin window mtf depth opt ptrmin mtfmin maxstream chunk pt pp mt mp\n");
        for (size_t i = 0; i < sizeof VECTORS / sizeof VECTORS[0]; ++i) {
            const vec_t *v = &VECTORS[i];
            uint8_t *in = generate(v->gen, v->size, v->seed);
            size_t clen; uint8_t *comp = build(v, in, &clen);
            uint8_t *back = calloc(1, v->size + 1);
            size_t r = decode_any(comp, clen, back, v->size, strcmp(v->kind, "stream") != 0);
            if (r != v->size || memcmp(back, in, v->size)) { fprintf(stderr, "%s: C round trip FAILED (%zu)\n", v->name, r); return 1; }
            snprintf(path, sizeof path, "%s/%s.x9", argv[2], v->name);
            FILE *f = fopen(path, "wb"); fwrite(comp, 1, clen, f); fclose(f);
            fprintf(man, "%s %s %s %zu %llu %u %u %u %u %u %u %u %u %zu %u %u %u %u\n", v->name, v->kind, v->gen, v->size,
                    (unsigned long long)v->seed, v->maxwin, v->window, v->mtf, v->depth, v->opt, v->ptrmin, v->mtfmin,
                    v->maxstream, v->chunk, v->pt, v->pp, v->mt, v->mp);
            printf("%-24s in=%8zu out=%8zu fnv=%016llx\n", v->name, v->size, clen, (unsigned long long)fnv1a(in, v->size));
            free(in); free(comp); free(back);
        }
        fclose(man);
        return 0;
    }
    if (argc >= 4 && (!strcmp(argv[1], "dec") || !strcmp(argv[1], "dm"))) {
        size_t n; uint8_t *comp = read_file(argv[2], &n);
        size_t want = (size_t)strtoull(argv[3], NULL, 10);
        uint8_t *out = calloc(1, want + 1);
        size_t r = decode_any(comp, n, out, want, !strcmp(argv[1], "dm"));
        if (r != want) { printf("FAIL produced=%zu\n", r); return 1; }
        printf("OK %zu fnv=%016llx\n", r, (unsigned long long)fnv1a(out, r));
        return 0;
    }
    fprintf(stderr, "usage: driver gen <dir> | dec <file> <len> | dm <file> <len>\n");
    return 2;
}
