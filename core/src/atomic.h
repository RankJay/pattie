/*
 * The core's only platform dependency: ordered access to Ring counters shared
 * with the app. Sequential tests can't verify ordering (ADR 0008); review any
 * change here against every branch below.
 *
 * MSVC (and clang-cl): the Windows driver and the Windows host tests.
 * GCC and Clang: the Linux backend, which embeds the core (ADR 0018).
 */
 #ifndef CORE_ATOMIC_H
 #define CORE_ATOMIC_H
 
 #include <stdint.h>
 
 #if defined(_MSC_VER)
 
 #if defined(_M_X64)
 /* The one intrinsic needed here, declared as <intrin.h> declares it, without
  * <intrin.h>: that drags in the kit's ammintrin.h, whose declarations drift from
  * newer compilers' built-ins (C4392 on __rmpadjust, WDK 10.0.28000 against the
  * windows-2025-vs2026 runner's MSVC, 2026-11-18), an error under /WX. */
 void _ReadWriteBarrier(void);
 #pragma intrinsic(_ReadWriteBarrier)
 
 /* x64 keeps loads and stores in order (TSO); only stop compiler reordering. */
 static __inline uint64_t load_acquire_u64(const volatile uint64_t *p)
 {
     uint64_t v = *p;
     _ReadWriteBarrier();
     return v;
 }
 
 static __inline void store_release_u64(volatile uint64_t *p, uint64_t v)
 {
     _ReadWriteBarrier();
     *p = v;
 }
 #elif defined(_M_ARM64)
 #include <intrin.h>
 
 static __inline uint64_t load_acquire_u64(const volatile uint64_t *p)
 {
     return __ldar64((unsigned __int64 volatile *)p);
 }
 
 static __inline void store_release_u64(volatile uint64_t *p, uint64_t v)
 {
     __stlr64((unsigned __int64 volatile *)p, v);
 }
 #else
 #error "Pattie core supports x64 and ARM64 only under MSVC"
 #endif
 
 #elif defined(__GNUC__) /* GCC, and Clang outside clang-cl */
 /* A counter is one 64-bit word the app reads or writes concurrently: its accesses
  * must be single atomic instructions, never a lock or two halves. */
 _Static_assert(__atomic_always_lock_free(sizeof(uint64_t), 0),
                "Pattie core needs lock-free 64-bit atomics");
 
 static inline uint64_t load_acquire_u64(const volatile uint64_t *p)
 {
     return __atomic_load_n(p, __ATOMIC_ACQUIRE);
 }
 
 static inline void store_release_u64(volatile uint64_t *p, uint64_t v)
 {
     __atomic_store_n(p, v, __ATOMIC_RELEASE);
 }
 
 #else
 #error "Pattie core needs MSVC, GCC or Clang"
 #endif
 
 #endif /* CORE_ATOMIC_H */
 