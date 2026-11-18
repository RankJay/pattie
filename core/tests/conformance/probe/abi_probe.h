/*
 * Test-only probes for the ABI mirror check (ADR 0012): exposes header
 * macros bindgen can't see, so the SDK's Rust mirror is compared with the real
 * C expansion rather than with a copy of the formula.
 */
#ifndef PATTIE_ABI_PROBE_H
#define PATTIE_ABI_PROBE_H

#include "pattie_abi.h"

uint32_t pattie_probe_ioctl(uint32_t command);

#endif /* PATTIE_ABI_PROBE_H */
