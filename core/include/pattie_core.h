/*
 * Pattie driver core: every decision the driver makes, in pure C (ADR 0002).
 *
 * No WDK headers, no locks, no allocation. The shell owns all storage, calls
 * the core one command at a time under its own lock, and carries out the result.
 * Tests reach the core only through the seams declared here (ADR 0008).
 *
 * Struct fields are internal; the definitions are public only so the shell can
 * embed them in its own contexts.
 */
#ifndef PATTIE_CORE_H
#define PATTIE_CORE_H

#include <stdbool.h>
#include <stddef.h>
#include "pattie_abi.h"

/*
 * The driver's private view of a Ring in shared memory. The core never trusts
 * what the app may change later.
 */
typedef struct pattie_ring {
    pattie_ring_header *shared;
    float *frames;
    uint32_t capacity_frames;
    uint32_t sample_rate;      /* read once at attach, like the capacity */
    uint32_t channels;         /* samples in a frame */
    uint64_t read_frames;      /* Microphone: the driver reads */
    uint64_t gap_frames;
    uint64_t write_frames;     /* Speaker: the driver writes */
    uint64_t overflow_frames;
    uint64_t clock_seq;        /* the published clock's sequence: never re-read */
} pattie_ring;

/* Per-Owner state. One per open handle, stored in the shell's file context. */
typedef struct pattie_owner {
    bool greeted;
    bool closed;    /* its handle closed: every later command is refused */
    uint16_t minor; /* the minor this connection speaks, once greeted (ADR 0003) */
} pattie_owner;

/* One live Device. A slot with id 0 is free. */
typedef struct pattie_device {
    uint32_t id;
    uint32_t kind;
    const pattie_owner *owner;
    uint32_t name_len;
    uint16_t name[PATTIE_DEVICE_NAME_MAX];
    uint64_t stable_id;        /* pattie_stable_id of kind, name and format, set at create */
    pattie_ring ring;
} pattie_device;

/* What the core accepted for a live Device (pattie_core_device_info). */
typedef struct pattie_device_info {
    uint32_t kind;
    uint32_t name_len;
    uint16_t name[PATTIE_DEVICE_NAME_MAX];
    uint64_t stable_id;
} pattie_device_info;

/* Driver-wide state. One per driver instance. */
typedef struct pattie_core {
    uint32_t next_device_id;
    pattie_device devices[PATTIE_MAX_DEVICES];
} pattie_core;

void pattie_core_init(pattie_core *core);
void pattie_owner_init(pattie_owner *owner);

/*
 * Commands. Runs one command exactly as the SDK sent it.
 *
 * For Create Device, the shell first pins the memory named by the request's
 * ring_address and ring_bytes (ADR 0006) and passes it as ring_memory and
 * ring_len; for every other command both are NULL and 0. If Create Device
 * fails, nothing refers to that memory and the shell unpins it.
 *
 * On return *output_len holds the number of reply bytes written to output.
 */
pattie_status pattie_core_command(pattie_core *core, pattie_owner *owner, uint32_t command,
                          const void *input, size_t input_len,
                          void *ring_memory, size_t ring_len,
                          void *output, size_t output_cap, size_t *output_len);

/*
 * Commands, continued: the Owner's handle closed (crash included). Removes every
 * Device it owned, writes their IDs to `removed` and returns how many, so the
 * shell can tear the Devices down and unpin their Rings. The Owner is marked
 * closed: a command of its still queued or in flight when the handle closed is
 * refused (PATTIE_ERR_BAD_REQUEST), so no Device outlives its Owner (security review
 * 2026-11-14). Only pattie_owner_init opens it again.
 */
size_t pattie_core_owner_closed(pattie_core *core, pattie_owner *owner,
                            uint32_t removed[PATTIE_MAX_DEVICES]);

/*
 * The Ring of a live Device, for the shell to stream through; NULL when no live
 * Device has that ID.
 */
pattie_ring *pattie_core_device_ring(pattie_core *core, uint32_t device_id);

/*
 * What the core accepted for a live Device: its kind, name and stable ID, so no
 * shell or host parses the request again. False (and *info untouched) when no
 * live Device has that ID.
 */
bool pattie_core_device_info(const pattie_core *core, uint32_t device_id, pattie_device_info *info);

/*
 * A Device's stable identity (ADR 0015, amended 2026-11-14): the 64-bit FNV-1a
 * hash of the kind byte (1 or 2), the name's UTF-16LE bytes, then the sample
 * rate (4 bytes) and channel count (2 bytes), little-endian; a hash of 0 becomes
 * 1, so 0 can mean "free". Names are case-sensitive (ADR 0004). The format is in
 * it because Windows saves a format per endpoint: the same name in another format
 * must be another endpoint, or Windows opens it in a format the pin no longer
 * offers. The one hash the system uses; the SDK's Rust copy
 * (`pattie::stable_id`) is checked against it (core/tests/conformance/tests/stable_id.rs).
 * How it's written is each host's: the driver's instance ID, PipeWire's
 * node.name.
 */
uint64_t pattie_stable_id(uint32_t kind, const uint16_t *name, uint32_t name_len,
                      uint32_t sample_rate, uint32_t channels);

/*
 * The Ring, driver side. Attaches to a Ring the SDK laid out at `memory`,
 * reading its capacity, sample rate and channels once: one of the PATTIE_RATE_*
 * rates, 1 to PATTIE_RING_MAX_CHANNELS channels (zeros mean 48 kHz mono), and memory
 * for the whole capacity of frames. Counts everywhere below are in frames.
 */
pattie_status pattie_ring_attach(pattie_ring *ring, void *memory, size_t memory_len);

/* Samples in each of this Ring's frames (its channels), as read at attach. */
uint32_t pattie_ring_channels(const pattie_ring *ring);

/* This Ring's sample rate in frames a second, as read at attach. */
uint32_t pattie_ring_sample_rate(const pattie_ring *ring);

/* Microphone direction: takes `count` frames out of the Ring into `dst`. */
void pattie_ring_read(pattie_ring *ring, float *dst, uint32_t count);

/* Speaker direction: puts `count` frames from `src` into the Ring. */
void pattie_ring_write(pattie_ring *ring, const float *src, uint32_t count);

/*
 * After a pass (ADR 0022): publishes the driver's counter and `now_ns` as the
 * Ring's clock pair, on the driver's own line: the read position for a
 * Microphone, the write position for a Speaker.
 */
void pattie_ring_publish_read_clock(pattie_ring *ring, uint64_t now_ns);
void pattie_ring_publish_write_clock(pattie_ring *ring, uint64_t now_ns);

/*
 * The Ring in Windows' format (ADR 0005): Windows' audio endpoint builder needs a
 * PCM device format, while the Ring and the app stay 32-bit float. Samples are
 * converted with integer arithmetic on their IEEE-754 bit patterns, so the driver
 * still has no floating-point code; samples cross this interface as bits.
 *
 * A float sample as 16-bit PCM: scaled by 32768, rounded half away from zero,
 * clamped to [-32768, 32767]. NaN, denormals and anything that rounds to zero give 0.
 */
int16_t pattie_pcm16_from_sample(uint32_t sample_bits);

/* A 16-bit PCM sample as a float sample's bits: exactly pcm / 32768. */
uint32_t pattie_sample_from_pcm16(int16_t pcm);

/* pattie_ring_read, delivering 16-bit PCM: same Gap and overrun behaviour. */
void pattie_ring_read_pcm16(pattie_ring *ring, int16_t *dst, uint32_t count);

/* pattie_ring_write, taking 16-bit PCM: same Overflow behaviour. */
void pattie_ring_write_pcm16(pattie_ring *ring, const int16_t *src, uint32_t count);

/*
 * Microphone direction, when a recording starts (ADR 0014): drops stale audio so
 * at most `keep_frames` of the newest stay unread, and returns how many frames it
 * dropped (for the shell's trace). Skipped frames are not counted as losses (not
 * a Gap, and the driver may not store to the writer's Overflow; ADR 0007).
 */
uint64_t pattie_ring_skip_stale(pattie_ring *ring, uint32_t keep_frames);

/* The driver side's loss totals so far: Gap (Microphone) and Overflow (Speaker). */
void pattie_ring_losses(const pattie_ring *ring, uint64_t *gap_frames, uint64_t *overflow_frames);

/*
 * The clock. Frames a Device has played between two tick readings
 * the shell took (ticks at `tick_hz`), at `sample_rate` frames per second.
 */
uint64_t pattie_clock_frames(uint64_t start_ticks, uint64_t now_ticks,
                         uint64_t tick_hz, uint32_t sample_rate);

/*
 * The clock: a tick reading at `tick_hz` in nanoseconds, rounded down: the unit a
 * Ring's clock pair is published in (ADR 0022).
 */
uint64_t pattie_clock_ns(uint64_t ticks, uint64_t tick_hz);

/*
 * The clock, inverse: the fewest ticks after the start at which pattie_clock_frames
 * reaches `frames`. The shell schedules packet completions with it.
 */
uint64_t pattie_clock_ticks_for(uint64_t frames, uint64_t tick_hz, uint32_t sample_rate);

/*
 * The clock, scheduling: when the pass completing packet `packets_done + 1` is due
 * (*due_ticks), counted from the stream's start (*start_ticks), for packets of
 * `packet_frames`. A stream a few passes behind catches up on its schedule. One
 * with more than `max_behind` passes already due was stalled (a sleep, a
 * suspended VM, a debugger): the audio it missed can't be recovered, so rather
 * than firing a pass for every missed packet it moves *start_ticks forward so
 * this pass is due now, as Microsoft's ACX stream engine does, and returns true.
 * Positions counted from the moved start continue from the packets completed.
 * Survives the tick counter wrapping.
 */
bool pattie_clock_next_pass(uint64_t *start_ticks, uint64_t packets_done, uint32_t packet_frames,
                        uint32_t max_behind, uint64_t now_ticks, uint64_t tick_hz,
                        uint32_t sample_rate, uint64_t *due_ticks);

#endif /* PATTIE_CORE_H */
