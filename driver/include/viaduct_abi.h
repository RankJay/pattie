/*
 * Viaduct ABI: the contract between the Windows driver and the SDK.
 *
 * Included by the C driver and, through bindgen, by the Rust SDK. Within a major
 * version this header only grows: never change or remove what a minor version
 * shipped (ADR 0003). No Windows or WDK headers may be included here.
 */
#ifndef VIADUCT_ABI_H
#define VIADUCT_ABI_H

#include <stdint.h>

#define VIADUCT_ABI_MAJOR 0u
#define VIADUCT_ABI_MINOR 1u

/* Result of every command. */
typedef uint32_t vd_status;
#define VD_OK 0u
#define VD_ERR_HELLO_REQUIRED 1u    /* Hello must be the Owner's first command */
#define VD_ERR_BUFFER_TOO_SMALL 2u  /* reply buffer can't hold the reply */
#define VD_ERR_BAD_RING 3u          /* Ring memory or header is invalid */
#define VD_ERR_BAD_REQUEST 4u       /* unknown command, or input of the wrong size */
#define VD_ERR_VERSION 5u           /* SDK's ABI version not served (ADR 0003) */
#define VD_ERR_BAD_KIND 6u          /* not a known Device kind */
#define VD_ERR_BAD_NAME 7u          /* empty, too long, or has control characters */
#define VD_ERR_DUPLICATE_NAME 8u    /* a live Device of this kind has that name */
#define VD_ERR_TOO_MANY_DEVICES 9u  /* VD_MAX_DEVICES are already live */
#define VD_ERR_NO_SUCH_DEVICE 10u   /* no live Device with that ID for this Owner */
#define VD_ERR_SYSTEM 11u           /* Windows refused to create the Device's endpoint */

/* Most Devices alive at once, across all Owners (ADR 0005). */
#define VD_MAX_DEVICES 8u

/* Commands an Owner can send. */
#define VD_CMD_HELLO 1u
#define VD_CMD_CREATE_DEVICE 2u
#define VD_CMD_REMOVE_DEVICE 3u

/*
 * How an Owner reaches the driver (ADR 0010): open this path, one handle per
 * Owner, then send each command as a buffered IOCTL.
 */
#define VIADUCT_DEVICE_PATH "\\\\.\\Viaduct"

/*
 * IOCTL code for a command: CTL_CODE(FILE_DEVICE_UNKNOWN, 0x800 + command,
 * METHOD_BUFFERED, FILE_READ_ACCESS | FILE_WRITE_ACCESS), spelled out so this
 * header needs no Windows headers.
 */
#define VD_IOCTL(command) ((0x22u << 16) | (3u << 14) | ((0x800u + (command)) << 2) | 0u)

/*
 * Every reply starts with this envelope, followed by the command's own reply
 * (if any). The IOCTL itself succeeds whenever the command ran; the outcome is
 * `status`. The output buffer must hold at least the envelope.
 */
typedef struct vd_reply_envelope {
    vd_status status;
} vd_reply_envelope;

typedef struct vd_hello_request {
    uint16_t major;
    uint16_t minor;
} vd_hello_request;

/* Device kinds. */
#define VD_KIND_MICROPHONE 1u
#define VD_KIND_SPEAKER 2u

/* Longest Device name, in UTF-16 code units. */
#define VD_DEVICE_NAME_MAX 63u

typedef struct vd_create_device_request {
    uint32_t kind;
    uint32_t name_len;                  /* UTF-16 code units used in name */
    uint16_t name[VD_DEVICE_NAME_MAX];
    uint16_t reserved;                  /* must be zero */
    uint64_t ring_address;              /* the app's Ring memory (ADR 0006) */
    uint64_t ring_bytes;                /* its size; the driver pins exactly this */
} vd_create_device_request;

/* Device IDs start at 1; 0 never names a Device. */
typedef struct vd_create_device_reply {
    uint32_t device_id;
} vd_create_device_reply;

typedef struct vd_remove_device_request {
    uint32_t device_id;
} vd_remove_device_request;

/*
 * Ring layout (ADR 0007). Lives in app-allocated memory the driver pins
 * (ADR 0006). Counters only grow and never wrap; each side stores only to its
 * own cache line. Audio frames (32-bit float, mono in v0) follow the header
 * immediately; frame n lives at index n % capacity_frames.
 */
/* Ring capacity: a power of two within these bounds (about 5 ms to 1.4 s at 48 kHz). */
#define VD_RING_MIN_FRAMES 256u
#define VD_RING_MAX_FRAMES 65536u

/* Largest Ring the driver will pin: the header plus VD_RING_MAX_FRAMES frames. */
#define VD_RING_MAX_BYTES (192u + VD_RING_MAX_FRAMES * 4u)

typedef struct vd_ring_header {
    /* Writer's line: only the writing side stores here. */
    uint64_t write_frames;
    uint64_t overflow_frames;
    uint8_t writer_pad[48];
    /* Reader's line: only the reading side stores here. */
    uint64_t read_frames;
    uint64_t gap_frames;
    uint8_t reader_pad[48];
    /* Fixed by the SDK before Create Device. */
    uint32_t capacity_frames;
    uint8_t config_pad[60];
} vd_ring_header;

#endif /* VIADUCT_ABI_H */
