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

#define INT8_MAX 127
#define INT16_MAX 32767
#define INT32_MAX 2147483647
#define INT64_MAX 9223372036854775807i64
#define UINT8_MAX 0xffu
#define UINT16_MAX 0xffffu
#define UINT32_MAX 0xffffffffu
#define UINT64_MAX 0xffffffffffffffffui64

#endif /* VIADUCT_KERNEL_STDINT_H */
