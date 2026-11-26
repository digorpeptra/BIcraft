/* GCC compatibility shim for building the MSVC-oriented XPress9 sources as a test oracle.
 * bicraft's own code (MIT OR Apache-2.0); force-included with -include shim.h. */
#ifndef BICRAFT_SHIM_H
#define BICRAFT_SHIM_H
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include <stdlib.h>

#define __int64 long long
#define __declspec(x)
#define __unaligned
#define __stdcall
#define XPRESS9_CALL
#define UNIX 1

/* behave like the x64 MSVC build, minus SSE intrinsics */
#define _M_AMD64 1
#define XPRESS9_USE_SSE2 0
#define XPRESS9_USE_SSE4 0
#define XPRESS9_MAX_TRACE_LEVEL 0

static inline unsigned char _BitScanReverse(unsigned long *index, unsigned long value)
{
    value &= 0xffffffffUL;
    if (value == 0) return 0;
    *index = 63 - __builtin_clzl(value);
    return 1;
}

extern unsigned long long g_fake_tsc;
static inline unsigned long long __rdtsc(void) { return g_fake_tsc; }

#endif
