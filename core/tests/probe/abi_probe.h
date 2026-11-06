/*
 * Test-only probes for the ABI mirror check (Seam S2, ADR 0012): exposes header
 * macros bindgen can't see, so the SDK's Rust mirror is compared with the real
 * C expansion rather than with a copy of the formula.
 */
#ifndef VIADUCT_ABI_PROBE_H
#define VIADUCT_ABI_PROBE_H

#include "viaduct_abi.h"

uint32_t vd_probe_ioctl(uint32_t command);

#endif /* VIADUCT_ABI_PROBE_H */
