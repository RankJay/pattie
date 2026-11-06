/*
 * The core's only platform dependency: ordered access to Ring counters shared
 * with the app. Sequential tests can't verify ordering (ADR 0008); review any
 * change here against both architectures.
 */
#ifndef VD_PORT_H
#define VD_PORT_H

#include <stdint.h>
#include <intrin.h>

#if defined(_M_X64)
/* x64 keeps loads and stores in order (TSO); only stop compiler reordering. */
static __inline uint64_t vd_load_acquire_u64(const volatile uint64_t *p)
{
    uint64_t v = *p;
    _ReadWriteBarrier();
    return v;
}

static __inline void vd_store_release_u64(volatile uint64_t *p, uint64_t v)
{
    _ReadWriteBarrier();
    *p = v;
}
#elif defined(_M_ARM64)
static __inline uint64_t vd_load_acquire_u64(const volatile uint64_t *p)
{
    return __ldar64((unsigned __int64 volatile *)p);
}

static __inline void vd_store_release_u64(volatile uint64_t *p, uint64_t v)
{
    __stlr64((unsigned __int64 volatile *)p, v);
}
#else
#error "Viaduct core supports x64 and ARM64 only"
#endif

#endif /* VD_PORT_H */
