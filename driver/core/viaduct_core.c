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
}

void vd_owner_init(vd_owner *owner)
{
    owner->greeted = false;
}

static vd_status create_device(vd_core *core, void *output, size_t output_cap,
                               size_t *output_len)
{
    vd_create_device_reply reply;

    if (output_cap < sizeof reply) {
        return VD_ERR_BUFFER_TOO_SMALL;
    }
    reply.device_id = core->next_device_id++;
    copy_bytes(output, &reply, sizeof reply);
    *output_len = sizeof reply;
    return VD_OK;
}

vd_status vd_core_command(vd_core *core, vd_owner *owner, uint32_t command,
                          const void *input, size_t input_len,
                          void *output, size_t output_cap, size_t *output_len)
{
    (void)input;
    *output_len = 0;

    if (command == VD_CMD_HELLO) {
        if (input_len != sizeof(vd_hello_request)) {
            return VD_ERR_BAD_REQUEST;
        }
        owner->greeted = true;
        return VD_OK;
    }
    if (!owner->greeted) {
        return VD_ERR_HELLO_REQUIRED;
    }
    if (command == VD_CMD_CREATE_DEVICE) {
        if (input_len != sizeof(vd_create_device_request)) {
            return VD_ERR_BAD_REQUEST;
        }
        return create_device(core, output, output_cap, output_len);
    }
    return VD_ERR_BAD_REQUEST;
}
