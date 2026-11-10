#include "viaduct_core.h"
#include "viaduct_port.h"

_Static_assert(sizeof(vd_ring_header) == 192, "Ring header is three cache lines");
_Static_assert(VD_RING_MAX_BYTES == sizeof(vd_ring_header) + VD_RING_MAX_FRAMES * sizeof(float),
               "VD_RING_MAX_BYTES matches the largest valid Ring");

/*
 * Samples are only ever copied, never computed on, so they move as raw 32-bit
 * patterns. That keeps every floating-point instruction out of the driver: no
 * FPU state to worry about at raised IRQL, and no _fltused on x64. Silence is
 * all-zero bits, which is exactly 0.0f.
 */
_Static_assert(sizeof(float) == sizeof(uint32_t), "samples are 32-bit");
typedef uint32_t vd_sample_bits;

vd_status vd_ring_attach(vd_ring *ring, void *memory, size_t memory_len)
{
    const vd_ring_header *header = (const vd_ring_header *)memory;
    uint32_t capacity;

    if (memory_len < sizeof(vd_ring_header)) {
        return VD_ERR_BAD_RING;
    }
    /* Read the claim exactly once: the app can rewrite it at any moment. */
    capacity = *(const volatile uint32_t *)&header->capacity_frames;
    if (capacity < VD_RING_MIN_FRAMES || capacity > VD_RING_MAX_FRAMES ||
        (capacity & (capacity - 1)) != 0) {
        return VD_ERR_BAD_RING;
    }
    if (memory_len - sizeof(vd_ring_header) < (size_t)capacity * sizeof(float)) {
        return VD_ERR_BAD_RING;
    }

    ring->shared = (vd_ring_header *)memory;
    ring->frames = (float *)((unsigned char *)memory + sizeof(vd_ring_header));
    ring->capacity_frames = capacity;
    ring->read_frames = 0;
    ring->gap_frames = 0;
    ring->write_frames = 0;
    ring->overflow_frames = 0;
    ring->clock_seq = 0;
    return VD_OK;
}

/*
 * The seqlock's writer. Every store is a release store, so each is ordered after
 * all the stores before it: a reader that sees the new pair saw `seq` go odd
 * first, and one that sees `seq` even again saw the whole pair.
 */
static void vd_publish_clock(vd_ring *ring, vd_ring_clock *clock, uint64_t frames,
                             uint64_t now_ns)
{
    ring->clock_seq += 2u; /* our own copy: the app can't skew it */
    vd_store_release_u64(&clock->seq, ring->clock_seq - 1u);
    vd_store_release_u64(&clock->frames, frames);
    vd_store_release_u64(&clock->ns, now_ns);
    vd_store_release_u64(&clock->seq, ring->clock_seq);
}

void vd_ring_publish_read_clock(vd_ring *ring, uint64_t now_ns)
{
    vd_publish_clock(ring, &ring->shared->reader_clock, ring->read_frames, now_ns);
}

void vd_ring_publish_write_clock(vd_ring *ring, uint64_t now_ns)
{
    vd_publish_clock(ring, &ring->shared->writer_clock, ring->write_frames, now_ns);
}

/*
 * How many of `count` frames the reader can take now. The writer counter is
 * app-controlled: never trust it further than this. An overrun skips ahead to
 * the oldest intact frame (ADR 0007).
 */
static uint32_t vd_ring_readable(vd_ring *ring, uint32_t count)
{
    uint64_t written = vd_load_acquire_u64(&ring->shared->write_frames);
    uint64_t available;

    if (written < ring->read_frames) {
        available = 0; /* moved backwards: nothing valid to read */
    } else {
        available = written - ring->read_frames;
        if (available > ring->capacity_frames) {
            /* The app overran its Ring; older frames were overwritten. */
            ring->read_frames = written - ring->capacity_frames;
            available = ring->capacity_frames;
        }
    }
    return available < count ? (uint32_t)available : count;
}

/* Publishes a read of `taken` frames out of `count` asked for; the rest were silence. */
static void vd_ring_read_done(vd_ring *ring, uint32_t taken, uint32_t count)
{
    ring->read_frames += taken;
    ring->gap_frames += count - taken;
    vd_store_release_u64(&ring->shared->gap_frames, ring->gap_frames);
    vd_store_release_u64(&ring->shared->read_frames, ring->read_frames);
}

/* How many of `count` frames fit now. The reader counter is app-controlled: clamp it. */
static uint32_t vd_ring_writable(vd_ring *ring, uint32_t count)
{
    uint64_t read = vd_load_acquire_u64(&ring->shared->read_frames);
    uint64_t unread = 0;
    uint64_t room;

    if (read <= ring->write_frames) {
        unread = ring->write_frames - read;
        if (unread > ring->capacity_frames) {
            unread = ring->capacity_frames; /* impossible claim: treat as full */
        }
    }
    room = ring->capacity_frames - unread;
    return room < count ? (uint32_t)room : count;
}

/* Publishes a write of `stored` frames out of `count` offered; the rest were dropped. */
static void vd_ring_write_done(vd_ring *ring, uint32_t stored, uint32_t count)
{
    ring->write_frames += stored;
    ring->overflow_frames += count - stored; /* no room: the newest frames are dropped */
    vd_store_release_u64(&ring->shared->overflow_frames, ring->overflow_frames);
    vd_store_release_u64(&ring->shared->write_frames, ring->write_frames);
}

void vd_ring_read(vd_ring *ring, float *dst, uint32_t count)
{
    uint32_t n = vd_ring_readable(ring, count);
    const vd_sample_bits *frames = (const vd_sample_bits *)ring->frames;
    vd_sample_bits *out = (vd_sample_bits *)dst;

    for (uint32_t i = 0; i < n; i++) {
        out[i] = frames[(ring->read_frames + i) % ring->capacity_frames];
    }
    for (uint32_t i = n; i < count; i++) {
        out[i] = 0; /* the app fell behind: listeners hear silence */
    }
    vd_ring_read_done(ring, n, count);
}

void vd_ring_read_pcm16(vd_ring *ring, int16_t *dst, uint32_t count)
{
    uint32_t n = vd_ring_readable(ring, count);
    const vd_sample_bits *frames = (const vd_sample_bits *)ring->frames;

    for (uint32_t i = 0; i < n; i++) {
        dst[i] = vd_pcm16_from_sample(frames[(ring->read_frames + i) % ring->capacity_frames]);
    }
    for (uint32_t i = n; i < count; i++) {
        dst[i] = 0;
    }
    vd_ring_read_done(ring, n, count);
}

void vd_ring_write(vd_ring *ring, const float *src, uint32_t count)
{
    uint32_t n = vd_ring_writable(ring, count);
    vd_sample_bits *frames = (vd_sample_bits *)ring->frames;
    const vd_sample_bits *in = (const vd_sample_bits *)src;

    for (uint32_t i = 0; i < n; i++) {
        frames[(ring->write_frames + i) % ring->capacity_frames] = in[i];
    }
    vd_ring_write_done(ring, n, count);
}

void vd_ring_write_pcm16(vd_ring *ring, const int16_t *src, uint32_t count)
{
    uint32_t n = vd_ring_writable(ring, count);
    vd_sample_bits *frames = (vd_sample_bits *)ring->frames;

    for (uint32_t i = 0; i < n; i++) {
        frames[(ring->write_frames + i) % ring->capacity_frames] = vd_sample_from_pcm16(src[i]);
    }
    vd_ring_write_done(ring, n, count);
}

void vd_ring_skip_stale(vd_ring *ring, uint32_t keep_frames)
{
    /* The writer counter is app-controlled: same rules as vd_ring_read. */
    uint64_t written = vd_load_acquire_u64(&ring->shared->write_frames);
    uint64_t keep = keep_frames < ring->capacity_frames ? keep_frames : ring->capacity_frames;

    if (written < ring->read_frames || written - ring->read_frames <= keep) {
        return; /* moved backwards, or nothing stale */
    }
    /* Audio written while nobody was recording: skip all but the newest `keep`. */
    ring->read_frames = written - keep;
    vd_store_release_u64(&ring->shared->read_frames, ring->read_frames);
}
