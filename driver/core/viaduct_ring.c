#include "viaduct_core.h"
#include "vd_port.h"

_Static_assert(sizeof(vd_ring_header) == 192, "Ring header is three cache lines");

vd_status vd_ring_attach(vd_ring *ring, void *memory, size_t memory_len)
{
    (void)memory_len;
    ring->shared = (vd_ring_header *)memory;
    ring->frames = (float *)((unsigned char *)memory + sizeof(vd_ring_header));
    ring->capacity_frames = ring->shared->capacity_frames;
    ring->read_frames = 0;
    ring->gap_frames = 0;
    return VD_OK;
}

void vd_ring_read(vd_ring *ring, float *dst, uint32_t count)
{
    uint64_t written = vd_load_acquire_u64(&ring->shared->write_frames);
    uint64_t available = written - ring->read_frames;
    uint32_t n = available < count ? (uint32_t)available : count;

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
