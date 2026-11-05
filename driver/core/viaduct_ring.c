#include "viaduct_core.h"
#include "vd_port.h"

_Static_assert(sizeof(vd_ring_header) == 192, "Ring header is three cache lines");

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
    return VD_OK;
}

void vd_ring_read(vd_ring *ring, float *dst, uint32_t count)
{
    /* The writer counter is app-controlled: never trust it further than this. */
    uint64_t written = vd_load_acquire_u64(&ring->shared->write_frames);
    uint64_t available;
    uint32_t n;

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
    n = available < count ? (uint32_t)available : count;

    for (uint32_t i = 0; i < n; i++) {
        dst[i] = ring->frames[(ring->read_frames + i) % ring->capacity_frames];
    }
    for (uint32_t i = n; i < count; i++) {
        dst[i] = 0.0f; /* the app fell behind: listeners hear silence */
    }
    ring->read_frames += n;
    ring->gap_frames += count - n;
    vd_store_release_u64(&ring->shared->gap_frames, ring->gap_frames);
    vd_store_release_u64(&ring->shared->read_frames, ring->read_frames);
}

void vd_ring_write(vd_ring *ring, const float *src, uint32_t count)
{
    /* The reader counter is app-controlled: clamp it to what can be true. */
    uint64_t read = vd_load_acquire_u64(&ring->shared->read_frames);
    uint64_t unread = 0;
    uint64_t room;
    uint32_t n;

    if (read <= ring->write_frames) {
        unread = ring->write_frames - read;
        if (unread > ring->capacity_frames) {
            unread = ring->capacity_frames; /* impossible claim: treat as full */
        }
    }
    room = ring->capacity_frames - unread;
    n = room < count ? (uint32_t)room : count;

    for (uint32_t i = 0; i < n; i++) {
        ring->frames[(ring->write_frames + i) % ring->capacity_frames] = src[i];
    }
    ring->write_frames += n;
    ring->overflow_frames += count - n; /* no room: the newest frames are dropped */
    vd_store_release_u64(&ring->shared->overflow_frames, ring->overflow_frames);
    vd_store_release_u64(&ring->shared->write_frames, ring->write_frames);
}
