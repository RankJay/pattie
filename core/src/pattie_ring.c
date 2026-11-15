#include "pattie_core.h"
#include "pattie_port.h"

_Static_assert(sizeof(pattie_ring_header) == 192, "Ring header is three cache lines");
_Static_assert(PATTIE_RING_MAX_BYTES == sizeof(pattie_ring_header) +
                                        PATTIE_RING_MAX_FRAMES * PATTIE_RING_MAX_CHANNELS * sizeof(float),
               "PATTIE_RING_MAX_BYTES matches the largest valid Ring");

/*
 * Samples are only ever copied, never computed on, so they move as raw 32-bit
 * patterns. That keeps every floating-point instruction out of the driver: no
 * FPU state to worry about at raised IRQL, and no _fltused on x64. Silence is
 * all-zero bits, which is exactly 0.0f.
 */
_Static_assert(sizeof(float) == sizeof(uint32_t), "samples are 32-bit");
typedef uint32_t pattie_sample_bits;

pattie_status pattie_ring_attach(pattie_ring *ring, void *memory, size_t memory_len)
{
    const pattie_ring_header *header = (const pattie_ring_header *)memory;
    uint32_t capacity;
    uint32_t rate;
    uint32_t channels;

    if (memory_len < sizeof(pattie_ring_header)) {
        return PATTIE_ERR_BAD_RING;
    }
    /* Read each claim exactly once: the app can rewrite them at any moment. */
    capacity = *(const volatile uint32_t *)&header->capacity_frames;
    rate = *(const volatile uint32_t *)&header->sample_rate;
    channels = *(const volatile uint16_t *)&header->channels;
    if (rate == 0u) {
        rate = PATTIE_RATE_48000;
    }
    if (channels == 0u) {
        channels = 1u;
    }
    if (capacity < PATTIE_RING_MIN_FRAMES || capacity > PATTIE_RING_MAX_FRAMES ||
        (capacity & (capacity - 1)) != 0) {
        return PATTIE_ERR_BAD_RING;
    }
    if (rate != PATTIE_RATE_44100 && rate != PATTIE_RATE_48000 && rate != PATTIE_RATE_96000) {
        return PATTIE_ERR_BAD_RING;
    }
    if (channels > PATTIE_RING_MAX_CHANNELS) {
        return PATTIE_ERR_BAD_RING;
    }
    /* At most 65,536 frames of 8 channels: no overflow in size_t. */
    if (memory_len - sizeof(pattie_ring_header) < (size_t)capacity * channels * sizeof(float)) {
        return PATTIE_ERR_BAD_RING;
    }

    ring->shared = (pattie_ring_header *)memory;
    ring->frames = (float *)((unsigned char *)memory + sizeof(pattie_ring_header));
    ring->capacity_frames = capacity;
    ring->sample_rate = rate;
    ring->channels = channels;
    ring->read_frames = 0;
    ring->gap_frames = 0;
    ring->write_frames = 0;
    ring->overflow_frames = 0;
    ring->clock_seq = 0;
    return PATTIE_OK;
}

/*
 * The seqlock's writer. Every store is a release store, so each is ordered after
 * all the stores before it: a reader that sees the new pair saw `seq` go odd
 * first, and one that sees `seq` even again saw the whole pair.
 */
static void pattie_publish_clock(pattie_ring *ring, pattie_ring_clock *clock, uint64_t frames,
                             uint64_t now_ns)
{
    ring->clock_seq += 2u; /* our own copy: the app can't skew it */
    pattie_store_release_u64(&clock->seq, ring->clock_seq - 1u);
    pattie_store_release_u64(&clock->frames, frames);
    pattie_store_release_u64(&clock->ns, now_ns);
    pattie_store_release_u64(&clock->seq, ring->clock_seq);
}

void pattie_ring_publish_read_clock(pattie_ring *ring, uint64_t now_ns)
{
    pattie_publish_clock(ring, &ring->shared->reader_clock, ring->read_frames, now_ns);
}

void pattie_ring_publish_write_clock(pattie_ring *ring, uint64_t now_ns)
{
    pattie_publish_clock(ring, &ring->shared->writer_clock, ring->write_frames, now_ns);
}

/*
 * How many of `count` frames the reader can take now. The writer counter is
 * app-controlled: never trust it further than this. An overrun skips ahead to
 * the oldest intact frame (ADR 0007).
 */
static uint32_t pattie_ring_readable(pattie_ring *ring, uint32_t count)
{
    uint64_t written = pattie_load_acquire_u64(&ring->shared->write_frames);
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
static void pattie_ring_read_done(pattie_ring *ring, uint32_t taken, uint32_t count)
{
    ring->read_frames += taken;
    ring->gap_frames += count - taken;
    pattie_store_release_u64(&ring->shared->gap_frames, ring->gap_frames);
    pattie_store_release_u64(&ring->shared->read_frames, ring->read_frames);
}

/* How many of `count` frames fit now. The reader counter is app-controlled: clamp it. */
static uint32_t pattie_ring_writable(pattie_ring *ring, uint32_t count)
{
    uint64_t read = pattie_load_acquire_u64(&ring->shared->read_frames);
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
static void pattie_ring_write_done(pattie_ring *ring, uint32_t stored, uint32_t count)
{
    ring->write_frames += stored;
    ring->overflow_frames += count - stored; /* no room: the newest frames are dropped */
    pattie_store_release_u64(&ring->shared->overflow_frames, ring->overflow_frames);
    pattie_store_release_u64(&ring->shared->write_frames, ring->write_frames);
}

uint32_t pattie_ring_channels(const pattie_ring *ring)
{
    return ring->channels;
}

uint32_t pattie_ring_sample_rate(const pattie_ring *ring)
{
    return ring->sample_rate;
}

/* The first sample of Ring frame `position` (a counter value). */
static size_t pattie_ring_sample(const pattie_ring *ring, uint64_t position)
{
    return (size_t)(position % ring->capacity_frames) * ring->channels;
}

/*
 * Each copy moves whole frames: `count` frames are `count * channels` samples,
 * interleaved, in the caller's buffer and in the Ring alike.
 */
void pattie_ring_read(pattie_ring *ring, float *dst, uint32_t count)
{
    uint32_t n = pattie_ring_readable(ring, count);
    uint32_t channels = ring->channels;
    const pattie_sample_bits *frames = (const pattie_sample_bits *)ring->frames;
    pattie_sample_bits *out = (pattie_sample_bits *)dst;

    for (uint32_t i = 0; i < n; i++) {
        size_t from = pattie_ring_sample(ring, ring->read_frames + i);
        for (uint32_t c = 0; c < channels; c++) {
            out[(size_t)i * channels + c] = frames[from + c];
        }
    }
    for (size_t i = (size_t)n * channels; i < (size_t)count * channels; i++) {
        out[i] = 0; /* the app fell behind: listeners hear silence */
    }
    pattie_ring_read_done(ring, n, count);
}

void pattie_ring_read_pcm16(pattie_ring *ring, int16_t *dst, uint32_t count)
{
    uint32_t n = pattie_ring_readable(ring, count);
    uint32_t channels = ring->channels;
    const pattie_sample_bits *frames = (const pattie_sample_bits *)ring->frames;

    for (uint32_t i = 0; i < n; i++) {
        size_t from = pattie_ring_sample(ring, ring->read_frames + i);
        for (uint32_t c = 0; c < channels; c++) {
            dst[(size_t)i * channels + c] = pattie_pcm16_from_sample(frames[from + c]);
        }
    }
    for (size_t i = (size_t)n * channels; i < (size_t)count * channels; i++) {
        dst[i] = 0;
    }
    pattie_ring_read_done(ring, n, count);
}

void pattie_ring_write(pattie_ring *ring, const float *src, uint32_t count)
{
    uint32_t n = pattie_ring_writable(ring, count);
    uint32_t channels = ring->channels;
    pattie_sample_bits *frames = (pattie_sample_bits *)ring->frames;
    const pattie_sample_bits *in = (const pattie_sample_bits *)src;

    for (uint32_t i = 0; i < n; i++) {
        size_t to = pattie_ring_sample(ring, ring->write_frames + i);
        for (uint32_t c = 0; c < channels; c++) {
            frames[to + c] = in[(size_t)i * channels + c];
        }
    }
    pattie_ring_write_done(ring, n, count);
}

void pattie_ring_write_pcm16(pattie_ring *ring, const int16_t *src, uint32_t count)
{
    uint32_t n = pattie_ring_writable(ring, count);
    uint32_t channels = ring->channels;
    pattie_sample_bits *frames = (pattie_sample_bits *)ring->frames;

    for (uint32_t i = 0; i < n; i++) {
        size_t to = pattie_ring_sample(ring, ring->write_frames + i);
        for (uint32_t c = 0; c < channels; c++) {
            frames[to + c] = pattie_sample_from_pcm16(src[(size_t)i * channels + c]);
        }
    }
    pattie_ring_write_done(ring, n, count);
}

uint64_t pattie_ring_skip_stale(pattie_ring *ring, uint32_t keep_frames)
{
    /* The writer counter is app-controlled: same rules as pattie_ring_read. */
    uint64_t written = pattie_load_acquire_u64(&ring->shared->write_frames);
    uint64_t keep = keep_frames < ring->capacity_frames ? keep_frames : ring->capacity_frames;
    uint64_t skipped;

    if (written < ring->read_frames || written - ring->read_frames <= keep) {
        return 0; /* moved backwards, or nothing stale */
    }
    /* Audio written while nobody was recording: skip all but the newest `keep`. */
    skipped = written - keep - ring->read_frames;
    ring->read_frames = written - keep;
    pattie_store_release_u64(&ring->shared->read_frames, ring->read_frames);
    return skipped;
}

void pattie_ring_losses(const pattie_ring *ring, uint64_t *gap_frames, uint64_t *overflow_frames)
{
    /* The driver's own copies, never the app's header. */
    *gap_frames = ring->gap_frames;
    *overflow_frames = ring->overflow_frames;
}
