#include "viaduct_core.h"

/* No libc in the kernel core: copy bytes by hand. Safe for unaligned buffers. */
static void copy_bytes(void *dst, const void *src, size_t len)
{
    unsigned char *d = (unsigned char *)dst;
    const unsigned char *s = (const unsigned char *)src;
    for (size_t i = 0; i < len; i++) {
        d[i] = s[i];
    }
}

void vd_core_init(vd_core *core)
{
    core->next_device_id = 1;
    for (uint32_t i = 0; i < VD_MAX_DEVICES; i++) {
        core->devices[i].id = 0;
    }
}

void vd_owner_init(vd_owner *owner)
{
    owner->greeted = false;
}

static bool is_control(uint16_t unit)
{
    return unit < 0x20 || (unit >= 0x7F && unit <= 0x9F);
}

static bool is_valid_name(const vd_create_device_request *request)
{
    if (request->name_len == 0 || request->name_len > VD_DEVICE_NAME_MAX) {
        return false;
    }
    for (uint32_t i = 0; i < request->name_len; i++) {
        if (is_control(request->name[i])) {
            return false;
        }
    }
    return true;
}

/* Exact, case-sensitive match (ADR 0004). */
static bool same_name(const vd_device *device, const vd_create_device_request *request)
{
    if (device->name_len != request->name_len) {
        return false;
    }
    for (uint32_t i = 0; i < request->name_len; i++) {
        if (device->name[i] != request->name[i]) {
            return false;
        }
    }
    return true;
}

static vd_status create_device(vd_core *core, const vd_owner *owner,
                               const vd_create_device_request *request,
                               void *ring_memory, size_t ring_len,
                               void *output, size_t output_cap, size_t *output_len)
{
    vd_create_device_reply reply;
    vd_device *free_slot = NULL;
    vd_ring ring;

    if (request->reserved != 0) {
        return VD_ERR_BAD_REQUEST;
    }
    if (request->kind != VD_KIND_MICROPHONE && request->kind != VD_KIND_SPEAKER) {
        return VD_ERR_BAD_KIND;
    }
    if (!is_valid_name(request)) {
        return VD_ERR_BAD_NAME;
    }
    for (uint32_t i = 0; i < VD_MAX_DEVICES; i++) {
        vd_device *device = &core->devices[i];
        if (device->id == 0) {
            if (free_slot == NULL) {
                free_slot = device;
            }
        } else if (device->kind == request->kind && same_name(device, request)) {
            return VD_ERR_DUPLICATE_NAME;
        }
    }
    if (free_slot == NULL) {
        return VD_ERR_TOO_MANY_DEVICES;
    }
    /* A Device is never live without its Ring (ADR 0007). */
    if (ring_memory == NULL || vd_ring_attach(&ring, ring_memory, ring_len) != VD_OK) {
        return VD_ERR_BAD_RING;
    }
    if (output_cap < sizeof reply) {
        return VD_ERR_BUFFER_TOO_SMALL;
    }

    free_slot->id = core->next_device_id++;
    free_slot->kind = request->kind;
    free_slot->owner = owner;
    free_slot->name_len = request->name_len;
    copy_bytes(free_slot->name, request->name, sizeof free_slot->name);
    free_slot->ring = ring;

    reply.device_id = free_slot->id;
    copy_bytes(output, &reply, sizeof reply);
    *output_len = sizeof reply;
    return VD_OK;
}

/* Another Owner's Device reads as missing, so IDs can't be probed. */
static vd_status remove_device(vd_core *core, const vd_owner *owner, uint32_t device_id)
{
    for (uint32_t i = 0; i < VD_MAX_DEVICES; i++) {
        vd_device *device = &core->devices[i];
        if (device_id != 0 && device->id == device_id && device->owner == owner) {
            device->id = 0;
            return VD_OK;
        }
    }
    return VD_ERR_NO_SUCH_DEVICE;
}

vd_status vd_core_command(vd_core *core, vd_owner *owner, uint32_t command,
                          const void *input, size_t input_len,
                          void *ring_memory, size_t ring_len,
                          void *output, size_t output_cap, size_t *output_len)
{
    *output_len = 0;

    if (command == VD_CMD_HELLO) {
        vd_hello_request hello;

        if (input_len != sizeof hello) {
            return VD_ERR_BAD_REQUEST;
        }
        copy_bytes(&hello, input, sizeof hello);
        if (hello.major != VIADUCT_ABI_MAJOR || hello.minor > VIADUCT_ABI_MINOR) {
            return VD_ERR_VERSION;
        }
        owner->greeted = true;
        return VD_OK;
    }
    if (!owner->greeted) {
        return VD_ERR_HELLO_REQUIRED;
    }
    if (command == VD_CMD_CREATE_DEVICE) {
        vd_create_device_request request;

        if (input_len != sizeof request) {
            return VD_ERR_BAD_REQUEST;
        }
        /* Copy first: the app could change shared input while we validate it. */
        copy_bytes(&request, input, sizeof request);
        return create_device(core, owner, &request, ring_memory, ring_len,
                             output, output_cap, output_len);
    }
    if (command == VD_CMD_REMOVE_DEVICE) {
        vd_remove_device_request request;

        if (input_len != sizeof request) {
            return VD_ERR_BAD_REQUEST;
        }
        copy_bytes(&request, input, sizeof request);
        return remove_device(core, owner, request.device_id);
    }
    return VD_ERR_BAD_REQUEST;
}

size_t vd_core_owner_closed(vd_core *core, const vd_owner *owner,
                            uint32_t removed[VD_MAX_DEVICES])
{
    size_t count = 0;

    for (uint32_t i = 0; i < VD_MAX_DEVICES; i++) {
        vd_device *device = &core->devices[i];
        if (device->id != 0 && device->owner == owner) {
            removed[count++] = device->id;
            device->id = 0;
        }
    }
    return count;
}

vd_ring *vd_core_device_ring(vd_core *core, uint32_t device_id)
{
    for (uint32_t i = 0; i < VD_MAX_DEVICES; i++) {
        vd_device *device = &core->devices[i];
        if (device_id != 0 && device->id == device_id) {
            return &device->ring;
        }
    }
    return NULL;
}
