/*
 * The core's only platform dependency: ordered access to Ring counters shared
 * with the app. Sequential tests can't verify ordering (ADR 0008); review any
 * change here against every branch below.
 *
 * MSVC (and clang-cl): the Windows driver and the Windows host tests.
 * GCC and Clang: the Linux backend, which embeds the core (ADR 0018).
 */
#ifndef VD_PORT_H
#define VD_PORT_H

#include <stdint.h>

#if defined(_MSC_VER)
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
#error "Viaduct core supports x64 and ARM64 only under MSVC"
#endif

#elif defined(__GNUC__) /* GCC, and Clang outside clang-cl */
/* A counter is one 64-bit word the app reads or writes concurrently: its accesses
 * must be single atomic instructions, never a lock or two halves. */
_Static_assert(__atomic_always_lock_free(sizeof(uint64_t), 0),
               "Viaduct core needs lock-free 64-bit atomics");

static inline uint64_t vd_load_acquire_u64(const volatile uint64_t *p)
{
    return __atomic_load_n(p, __ATOMIC_ACQUIRE);
}

static inline void vd_store_release_u64(volatile uint64_t *p, uint64_t v)
{
    __atomic_store_n(p, v, __ATOMIC_RELEASE);
}

#else
#error "Viaduct core needs MSVC, GCC or Clang"
#endif

#endif /* VD_PORT_H */
