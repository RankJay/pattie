/*
 * Kernel-mode stand-in for <stdint.h>.
 *
 * The WDK's kernel C runtime has no stdint.h, and the MSVC one pulls in the
 * user-mode vcruntime, which clashes with the kernel CRT. The shell project puts
 * this folder first on the include path so the pure core and the shared ABI
 * header (which include <stdint.h>) build unchanged in the driver. Host builds
 * never see this file.
 */
#ifndef VIADUCT_KERNEL_STDINT_H
#define VIADUCT_KERNEL_STDINT_H

#ifndef _KERNEL_MODE
#error "kernel-crt/stdint.h is for kernel-mode builds only"
#endif

typedef signed __int8 int8_t;
typedef signed __int16 int16_t;
typedef signed __int32 int32_t;
typedef signed __int64 int64_t;
typedef unsigned __int8 uint8_t;
typedef unsigned __int16 uint16_t;
typedef unsigned __int32 uint32_t;
typedef unsigned __int64 uint64_t;

/* ntintsafe.h defines some of these too: only fill in what's missing. */
#ifndef INT8_MAX
#define INT8_MAX 127
#endif
#ifndef INT16_MAX
#define INT16_MAX 32767
#endif
#ifndef INT32_MAX
#define INT32_MAX 2147483647
#endif
#ifndef INT64_MAX
#define INT64_MAX 9223372036854775807i64
#endif
#ifndef UINT8_MAX
#define UINT8_MAX 0xffu
#endif
#ifndef UINT16_MAX
#define UINT16_MAX 0xffffu
#endif
#ifndef UINT32_MAX
#define UINT32_MAX 0xffffffffu
#endif
#ifndef UINT64_MAX
#define UINT64_MAX 0xffffffffffffffffui64
#endif

#endif /* VIADUCT_KERNEL_STDINT_H */
