/*
 * Kernel-mode stand-in for <stdbool.h>; see stdint.h in this folder for why.
 * C17's _Bool is built in, so this is only the standard macros.
 */
#ifndef PATTIE_KERNEL_STDBOOL_H
#define PATTIE_KERNEL_STDBOOL_H

#ifndef _KERNEL_MODE
#error "kernel-crt/stdbool.h is for kernel-mode builds only"
#endif

#define bool _Bool
#define true 1
#define false 0
#define __bool_true_false_are_defined 1

#endif /* PATTIE_KERNEL_STDBOOL_H */
