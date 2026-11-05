/*
 * Viaduct driver core: every decision the driver makes, in pure C (ADR 0002).
 *
 * No WDK headers, no locks, no allocation. The shell owns all storage, calls
 * the core one command at a time under its own lock, and carries out the result.
 * Tests reach the core only through the seams declared here (ADR 0008).
 *
 * Struct fields are internal; the definitions are public only so the shell can
 * embed them in its own contexts.
 */
#ifndef VIADUCT_CORE_H
#define VIADUCT_CORE_H

#include <stdbool.h>
#include <stddef.h>
#include "viaduct_abi.h"

/* Driver-wide state. One per driver instance. */
typedef struct vd_core {
    uint32_t next_device_id;
} vd_core;

/* Per-Owner state. One per open handle, stored in the shell's file context. */
typedef struct vd_owner {
    bool greeted;
} vd_owner;

void vd_core_init(vd_core *core);
void vd_owner_init(vd_owner *owner);

/*
 * Seam A: the control path. Runs one command exactly as the SDK sent it.
 * On return *output_len holds the number of reply bytes written to output.
 */
vd_status vd_core_command(vd_core *core, vd_owner *owner, uint32_t command,
                          const void *input, size_t input_len,
                          void *output, size_t output_cap, size_t *output_len);

/*
 * Seam B: the Ring, driver side. `vd_ring` is the driver's private view of a
 * Ring in shared memory; the core never trusts what the app may change later.
 */
typedef struct vd_ring {
    vd_ring_header *shared;
    float *frames;
    uint32_t capacity_frames;
    uint64_t read_frames;
    uint64_t gap_frames;
} vd_ring;

/* Attaches to a Ring the SDK laid out at `memory`. */
vd_status vd_ring_attach(vd_ring *ring, void *memory, size_t memory_len);

/* Microphone direction: takes `count` frames out of the Ring into `dst`. */
void vd_ring_read(vd_ring *ring, float *dst, uint32_t count);

/*
 * Seam C: the clock. Frames a Device has played between two tick readings
 * the shell took (ticks at `tick_hz`), at `sample_rate` frames per second.
 */
uint64_t vd_clock_frames(uint64_t start_ticks, uint64_t now_ticks,
                         uint64_t tick_hz, uint32_t sample_rate);

#endif /* VIADUCT_CORE_H */
