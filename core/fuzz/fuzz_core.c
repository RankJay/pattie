/*
 * Fuzz target for the driver core (ADR 0002, ADR 0007): libFuzzer feeds bytes,
 * read here as a script of what hostile apps could do, and AddressSanitizer stops
 * at the first out-of-bounds or use-after-free access.
 *
 * Every Ring is a malloc of exactly the size the driver is told, so a read or
 * write one byte past it is caught. Apps send request bytes straight from the
 * input, rewrite their Rings' headers between driver calls, and close at random.
 *
 * Build and run: README, "Fuzzing the core". Host-only: nothing here loads the
 * driver.
 */
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include "viaduct_core.h"

#define OWNERS 3u
#define MAX_STEPS 64u
#define MAX_FRAMES 4096u /* per driver call, above the largest Ring a test makes */

typedef struct cursor {
    const uint8_t *data;
    size_t size;
    size_t pos;
} cursor;

/* Next `len` input bytes into `out`; zeros once the input runs out. */
static void take(cursor *c, void *out, size_t len)
{
    uint8_t *dst = (uint8_t *)out;
    for (size_t i = 0; i < len; i++) {
        dst[i] = c->pos < c->size ? c->data[c->pos++] : 0u;
    }
}

static uint8_t take_u8(cursor *c)
{
    uint8_t v;
    take(c, &v, sizeof v);
    return v;
}

static uint16_t take_u16(cursor *c)
{
    uint16_t v;
    take(c, &v, sizeof v);
    return v;
}

static uint32_t take_u32(cursor *c)
{
    uint32_t v;
    take(c, &v, sizeof v);
    return v;
}

/* The test's side of the shell: pinned Ring memory per live Device. */
typedef struct pinned {
    uint32_t id;
    void *memory;
} pinned;

typedef struct world {
    vd_core core;
    vd_owner owners[OWNERS];
    pinned rings[VD_MAX_DEVICES];
} world;

static void pin(world *w, uint32_t id, void *memory)
{
    for (uint32_t i = 0; i < VD_MAX_DEVICES; i++) {
        if (w->rings[i].id == 0) {
            w->rings[i].id = id;
            w->rings[i].memory = memory;
            return;
        }
    }
    abort(); /* more live Devices than the core allows: a core bug */
}

static void unpin(world *w, uint32_t id)
{
    for (uint32_t i = 0; i < VD_MAX_DEVICES; i++) {
        if (w->rings[i].id == id) {
            free(w->rings[i].memory);
            w->rings[i].id = 0;
            w->rings[i].memory = NULL;
            return;
        }
    }
    abort(); /* the core removed a Device the test never saw created */
}

/* Ring sizes to offer: the valid range, plus nonsense. */
static size_t ring_len_from(uint8_t choice, uint32_t *capacity_claim)
{
    static const uint32_t claims[] = {256u, 512u, 1024u, 0u, 255u, 300u, 65537u, 0xFFFFFFFFu};
    uint32_t claim = claims[choice % (sizeof claims / sizeof claims[0])];
    size_t exact = sizeof(vd_ring_header) + (size_t)(claim > 65536u ? 0u : claim) * sizeof(float);

    *capacity_claim = claim;
    switch ((choice >> 3) % 4u) {
    case 0: return exact;
    case 1: return exact > 0 ? exact - 1u : 0u; /* one byte short */
    case 2: return exact + 4u;                  /* slack after the frames */
    default: return sizeof(vd_ring_header) - 1u; /* not even a header */
    }
}

static void create(world *w, cursor *c, vd_owner *owner)
{
    vd_create_device_request request;
    uint32_t claim;
    size_t ring_len = ring_len_from(take_u8(c), &claim);
    void *memory = malloc(ring_len > 0 ? ring_len : 1u);
    uint8_t reply[sizeof(vd_create_device_reply)];
    size_t reply_cap = (take_u8(c) & 1u) ? sizeof reply : 2u;
    size_t reply_len = 0;

    if (memory == NULL) {
        return;
    }
    memset(memory, 0, ring_len > 0 ? ring_len : 1u);
    if (ring_len >= sizeof(vd_ring_header)) {
        ((vd_ring_header *)memory)->capacity_frames = claim;
    }
    take(c, &request, sizeof request); /* every field straight from the input */
    if (vd_core_command(&w->core, owner, VD_CMD_CREATE_DEVICE, &request, sizeof request,
                        memory, ring_len, reply, reply_cap, &reply_len) == VD_OK) {
        vd_create_device_reply created;
        if (reply_len != sizeof created) {
            abort();
        }
        memcpy(&created, reply, sizeof created);
        pin(w, created.device_id, memory);
    } else {
        free(memory); /* a failed create keeps no reference (viaduct_core.h) */
    }
}

static void close_owner(world *w, vd_owner *owner)
{
    uint32_t removed[VD_MAX_DEVICES];
    size_t count = vd_core_owner_closed(&w->core, owner, removed);

    for (size_t i = 0; i < count; i++) {
        unpin(w, removed[i]);
    }
    vd_owner_init(owner); /* the handle is reopened as a new Owner */
}

/* Windows streams through a live Device, with buffers of exactly `count`. */
static void stream(world *w, cursor *c)
{
    pinned *slot = &w->rings[take_u8(c) % VD_MAX_DEVICES];
    uint8_t op = take_u8(c);
    uint32_t count = take_u16(c) % (MAX_FRAMES + 1u);
    vd_ring *ring;
    void *buffer;

    if (slot->id == 0 || (ring = vd_core_device_ring(&w->core, slot->id)) == NULL) {
        return;
    }
    buffer = malloc(count > 0 ? (size_t)count * sizeof(float) : 1u);
    if (buffer == NULL) {
        return;
    }
    memset(buffer, 0, count > 0 ? (size_t)count * sizeof(float) : 1u);
    switch (op % 5u) {
    case 0: vd_ring_read(ring, (float *)buffer, count); break;
    case 1: vd_ring_read_pcm16(ring, (int16_t *)buffer, count); break;
    case 2: vd_ring_write(ring, (const float *)buffer, count); break;
    case 3: vd_ring_write_pcm16(ring, (const int16_t *)buffer, count); break;
    default: vd_ring_skip_stale(ring, count); break;
    }
    free(buffer);
}

/* The app rewrites part of a live Ring's header (or its frames) at will. */
static void scribble(world *w, cursor *c)
{
    pinned *slot = &w->rings[take_u8(c) % VD_MAX_DEVICES];
    uint8_t offset = take_u8(c) % sizeof(vd_ring_header);
    uint8_t len = take_u8(c) % 16u;

    if (slot->id == 0) {
        return;
    }
    if (offset + len > sizeof(vd_ring_header)) {
        len = (uint8_t)(sizeof(vd_ring_header) - offset);
    }
    take(c, (uint8_t *)slot->memory + offset, len);
}

int LLVMFuzzerTestOneInput(const uint8_t *data, size_t size);

int LLVMFuzzerTestOneInput(const uint8_t *data, size_t size)
{
    static world w; /* static: vd_core holds 8 Devices with their names */
    cursor c = {data, size, 0};

    memset(&w, 0, sizeof w);
    vd_core_init(&w.core);
    for (uint32_t i = 0; i < OWNERS; i++) {
        vd_owner_init(&w.owners[i]);
    }

    for (uint32_t step = 0; step < MAX_STEPS && c.pos < c.size; step++) {
        uint8_t op = take_u8(&c);
        vd_owner *owner = &w.owners[take_u8(&c) % OWNERS];
        uint8_t input[sizeof(vd_create_device_request) + 8u];
        uint8_t output[16];
        size_t output_len = 0;

        switch (op % 7u) {
        case 0: { /* Hello, mostly the served version */
            vd_hello_request hello = {VIADUCT_ABI_MAJOR, VIADUCT_ABI_MINOR};
            if (take_u8(&c) & 1u) {
                take(&c, &hello, sizeof hello);
            }
            (void)vd_core_command(&w.core, owner, VD_CMD_HELLO, &hello, sizeof hello,
                                  NULL, 0, output, sizeof output, &output_len);
            break;
        }
        case 1:
            create(&w, &c, owner);
            break;
        case 2: { /* Remove, with an ID from the input */
            vd_remove_device_request remove = {take_u32(&c) % 12u};
            (void)vd_core_command(&w.core, owner, VD_CMD_REMOVE_DEVICE, &remove, sizeof remove,
                                  NULL, 0, output, sizeof output, &output_len);
            for (uint32_t i = 0; i < VD_MAX_DEVICES; i++) {
                uint32_t id = w.rings[i].id;
                if (id != 0 && vd_core_device_ring(&w.core, id) == NULL) {
                    unpin(&w, id); /* removed: the shell unpins */
                }
            }
            break;
        }
        case 3: { /* Any command, any length, any bytes */
            uint32_t command = take_u32(&c);
            size_t len = take_u8(&c) % (sizeof input + 1u);
            take(&c, input, len);
            (void)vd_core_command(&w.core, owner, command, input, len,
                                  NULL, 0, output, take_u8(&c) % (sizeof output + 1u), &output_len);
            if (output_len > sizeof output) {
                abort(); /* the core claimed more reply than the buffer holds */
            }
            break;
        }
        case 4:
            close_owner(&w, owner);
            break;
        case 5:
            stream(&w, &c);
            break;
        default:
            scribble(&w, &c);
            break;
        }
    }

    for (uint32_t i = 0; i < OWNERS; i++) {
        close_owner(&w, &w.owners[i]);
    }
    for (uint32_t i = 0; i < VD_MAX_DEVICES; i++) {
        if (w.rings[i].id != 0) {
            abort(); /* closing every Owner left a Device live: a core bug */
        }
    }
    return 0;
}
