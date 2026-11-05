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

/*
 * The driver's private view of a Ring in shared memory. The core never trusts
 * what the app may change later.
 */
typedef struct vd_ring {
    vd_ring_header *shared;
    float *frames;
    uint32_t capacity_frames;
    uint64_t read_frames;      /* Microphone: the driver reads */
    uint64_t gap_frames;
    uint64_t write_frames;     /* Speaker: the driver writes */
    uint64_t overflow_frames;
} vd_ring;

/* Per-Owner state. One per open handle, stored in the shell's file context. */
typedef struct vd_owner {
    bool greeted;
} vd_owner;

/* One live Device. A slot with id 0 is free. */
typedef struct vd_device {
    uint32_t id;
    uint32_t kind;
    const vd_owner *owner;
    uint32_t name_len;
    uint16_t name[VD_DEVICE_NAME_MAX];
    vd_ring ring;
} vd_device;

/* Driver-wide state. One per driver instance. */
typedef struct vd_core {
    uint32_t next_device_id;
    vd_device devices[VD_MAX_DEVICES];
} vd_core;

void vd_core_init(vd_core *core);
void vd_owner_init(vd_owner *owner);

/*
 * Seam A: the control path. Runs one command exactly as the SDK sent it.
 *
 * For Create Device, the shell first pins the memory named by the request's
 * ring_address and ring_bytes (ADR 0006) and passes it as ring_memory and
 * ring_len; for every other command both are NULL and 0. If Create Device
 * fails, nothing refers to that memory and the shell unpins it.
 *
 * On return *output_len holds the number of reply bytes written to output.
 */
vd_status vd_core_command(vd_core *core, vd_owner *owner, uint32_t command,
                          const void *input, size_t input_len,
                          void *ring_memory, size_t ring_len,
                          void *output, size_t output_cap, size_t *output_len);

/*
 * Seam A, continued: the Owner's handle closed (crash included). Removes every
 * Device it owned, writes their IDs to `removed` and returns how many, so the
 * shell can tear the Devices down and unpin their Rings.
 */
size_t vd_core_owner_closed(vd_core *core, const vd_owner *owner,
                            uint32_t removed[VD_MAX_DEVICES]);

/*
 * The Ring of a live Device, for the shell to stream through; NULL when no live
 * Device has that ID.
 */
vd_ring *vd_core_device_ring(vd_core *core, uint32_t device_id);

/* Seam B: the Ring, driver side. Attaches to a Ring the SDK laid out at `memory`. */
vd_status vd_ring_attach(vd_ring *ring, void *memory, size_t memory_len);

/* Microphone direction: takes `count` frames out of the Ring into `dst`. */
void vd_ring_read(vd_ring *ring, float *dst, uint32_t count);

/* Speaker direction: puts `count` frames from `src` into the Ring. */
void vd_ring_write(vd_ring *ring, const float *src, uint32_t count);

/*
 * Seam C: the clock. Frames a Device has played between two tick readings
 * the shell took (ticks at `tick_hz`), at `sample_rate` frames per second.
 */
uint64_t vd_clock_frames(uint64_t start_ticks, uint64_t now_ticks,
                         uint64_t tick_hz, uint32_t sample_rate);

/*
 * Seam C, inverse: the fewest ticks after the start at which vd_clock_frames
 * reaches `frames`. The shell schedules packet completions with it.
 */
uint64_t vd_clock_ticks_for(uint64_t frames, uint64_t tick_hz, uint32_t sample_rate);

#endif /* VIADUCT_CORE_H */
