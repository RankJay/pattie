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
    uint32_t sample_rate;      /* read once at attach, like the capacity */
    uint32_t channels;         /* samples in a frame */
    uint64_t read_frames;      /* Microphone: the driver reads */
    uint64_t gap_frames;
    uint64_t write_frames;     /* Speaker: the driver writes */
    uint64_t overflow_frames;
    uint64_t clock_seq;        /* the published clock's sequence: never re-read */
} vd_ring;

/* Per-Owner state. One per open handle, stored in the shell's file context. */
typedef struct vd_owner {
    bool greeted;
    uint16_t minor; /* the minor this connection speaks, once greeted (ADR 0003) */
} vd_owner;

/* One live Device. A slot with id 0 is free. */
typedef struct vd_device {
    uint32_t id;
    uint32_t kind;
    const vd_owner *owner;
    uint32_t name_len;
    uint16_t name[VD_DEVICE_NAME_MAX];
    uint64_t stable_id;        /* vd_stable_id of kind, name and format, set at create */
    vd_ring ring;
} vd_device;

/* What the core accepted for a live Device (vd_core_device_info). */
typedef struct vd_device_info {
    uint32_t kind;
    uint32_t name_len;
    uint16_t name[VD_DEVICE_NAME_MAX];
    uint64_t stable_id;
} vd_device_info;

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

/*
 * What the core accepted for a live Device: its kind, name and stable ID, so no
 * shell or host parses the request again. False (and *info untouched) when no
 * live Device has that ID.
 */
bool vd_core_device_info(const vd_core *core, uint32_t device_id, vd_device_info *info);

/*
 * A Device's stable identity (ADR 0015, amended 2026-11-14): the 64-bit FNV-1a
 * hash of the kind byte (1 or 2), the name's UTF-16LE bytes, then the sample
 * rate (4 bytes) and channel count (2 bytes), little-endian; a hash of 0 becomes
 * 1, so 0 can mean "free". Names are case-sensitive (ADR 0004). The format is in
 * it because Windows saves a format per endpoint: the same name in another format
 * must be another endpoint, or Windows opens it in a format the pin no longer
 * offers. The one hash the system uses; the SDK's Rust copy
 * (`viaduct::stable_id`) is checked against it (core/tests/tests/stable_id.rs).
 * How it's written is each host's: the driver's instance ID, PipeWire's
 * node.name.
 */
uint64_t vd_stable_id(uint32_t kind, const uint16_t *name, uint32_t name_len,
                      uint32_t sample_rate, uint32_t channels);

/*
 * Seam B: the Ring, driver side. Attaches to a Ring the SDK laid out at `memory`,
 * reading its capacity, sample rate and channels once: one of the VD_RATE_*
 * rates, 1 to VD_RING_MAX_CHANNELS channels (zeros mean 48 kHz mono), and memory
 * for the whole capacity of frames. Counts everywhere below are in frames.
 */
vd_status vd_ring_attach(vd_ring *ring, void *memory, size_t memory_len);

/* Samples in each of this Ring's frames (its channels), as read at attach. */
uint32_t vd_ring_channels(const vd_ring *ring);

/* This Ring's sample rate in frames a second, as read at attach. */
uint32_t vd_ring_sample_rate(const vd_ring *ring);

/* Microphone direction: takes `count` frames out of the Ring into `dst`. */
void vd_ring_read(vd_ring *ring, float *dst, uint32_t count);

/* Speaker direction: puts `count` frames from `src` into the Ring. */
void vd_ring_write(vd_ring *ring, const float *src, uint32_t count);

/*
 * After a pass (ADR 0022): publishes the driver's counter and `now_ns` as the
 * Ring's clock pair, on the driver's own line: the read position for a
 * Microphone, the write position for a Speaker.
 */
void vd_ring_publish_read_clock(vd_ring *ring, uint64_t now_ns);
void vd_ring_publish_write_clock(vd_ring *ring, uint64_t now_ns);

/*
 * Seam B, Windows-facing format (ADR 0005): Windows' audio endpoint builder needs a
 * PCM device format, while the Ring and the app stay 32-bit float. Samples are
 * converted with integer arithmetic on their IEEE-754 bit patterns, so the driver
 * still has no floating-point code; samples cross this interface as bits.
 *
 * A float sample as 16-bit PCM: scaled by 32768, rounded half away from zero,
 * clamped to [-32768, 32767]. NaN, denormals and anything that rounds to zero give 0.
 */
int16_t vd_pcm16_from_sample(uint32_t sample_bits);

/* A 16-bit PCM sample as a float sample's bits: exactly pcm / 32768. */
uint32_t vd_sample_from_pcm16(int16_t pcm);

/* vd_ring_read, delivering 16-bit PCM: same Gap and overrun behaviour. */
void vd_ring_read_pcm16(vd_ring *ring, int16_t *dst, uint32_t count);

/* vd_ring_write, taking 16-bit PCM: same Overflow behaviour. */
void vd_ring_write_pcm16(vd_ring *ring, const int16_t *src, uint32_t count);

/*
 * Microphone direction, when a recording starts (ADR 0014): drops stale audio so
 * at most `keep_frames` of the newest stay unread, and returns how many frames it
 * dropped (for the shell's trace). Skipped frames are not counted as losses (not
 * a Gap, and the driver may not store to the writer's Overflow; ADR 0007).
 */
uint64_t vd_ring_skip_stale(vd_ring *ring, uint32_t keep_frames);

/* The driver side's loss totals so far: Gap (Microphone) and Overflow (Speaker). */
void vd_ring_losses(const vd_ring *ring, uint64_t *gap_frames, uint64_t *overflow_frames);

/*
 * Seam C: the clock. Frames a Device has played between two tick readings
 * the shell took (ticks at `tick_hz`), at `sample_rate` frames per second.
 */
uint64_t vd_clock_frames(uint64_t start_ticks, uint64_t now_ticks,
                         uint64_t tick_hz, uint32_t sample_rate);

/*
 * Seam C: a tick reading at `tick_hz` in nanoseconds, rounded down: the unit a
 * Ring's clock pair is published in (ADR 0022).
 */
uint64_t vd_clock_ns(uint64_t ticks, uint64_t tick_hz);

/*
 * Seam C, inverse: the fewest ticks after the start at which vd_clock_frames
 * reaches `frames`. The shell schedules packet completions with it.
 */
uint64_t vd_clock_ticks_for(uint64_t frames, uint64_t tick_hz, uint32_t sample_rate);

/*
 * Seam C, scheduling: when the pass completing packet `packets_done + 1` is due
 * (*due_ticks), counted from the stream's start (*start_ticks), for packets of
 * `packet_frames`. A stream a few passes behind catches up on its schedule. One
 * with more than `max_behind` passes already due was stalled (a sleep, a
 * suspended VM, a debugger): the audio it missed can't be recovered, so rather
 * than firing a pass for every missed packet it moves *start_ticks forward so
 * this pass is due now, as Microsoft's ACX stream engine does, and returns true.
 * Positions counted from the moved start continue from the packets completed.
 * Survives the tick counter wrapping.
 */
bool vd_clock_next_pass(uint64_t *start_ticks, uint64_t packets_done, uint32_t packet_frames,
                        uint32_t max_behind, uint64_t now_ticks, uint64_t tick_hz,
                        uint32_t sample_rate, uint64_t *due_ticks);

#endif /* VIADUCT_CORE_H */
