/*
 * Viaduct driver shell: shared declarations.
 *
 * The shell only translates between Windows and the pure core (ADR 0002): every
 * decision lives in viaduct_core.h. Layout (ADR 0010):
 *
 *   root ACX device   installed by the INF; parents the circuit devices
 *   control device    \\.\Viaduct, where Owners send commands (ADR 0011 access)
 *   circuit devices   one raw child per Device, carrying one capture or render
 *                     circuit, added and removed at runtime
 *
 * Files, one concern each:
 *
 *   driver.c      DriverEntry and the root ACX device
 *   control.c     the control device: Owners, commands, Ring pinning
 *   endpoint.c    a circuit device's lifecycle: create, prepare, name, tear down
 *   circuit.c     the circuit inside it: pins and the formats Windows may use
 *   interfaces.c  the audio interface values an INF would normally write
 *   stream.c      streaming: one copy per packet between Windows and the Ring
 *
 * Lifetime rule: everything touching a Device's Ring from the streaming timer
 * holds the circuit device's rundown protection. Removal runs it down, then
 * unpins the Ring, all under the driver lock, so no Ring memory is touched after
 * it is unpinned and no core slot is reused mid-removal.
 */
#ifndef VIADUCT_SHELL_H
#define VIADUCT_SHELL_H

#include <initguid.h>
#include <ntddk.h>
#include <windef.h>
#include <mmsystem.h>
#define NOBITMAP            /* mmreg.h: skip video types a kernel audio driver lacks */
#include <mmreg.h>          /* before ksmedia.h: enables the IEEE-float format GUIDs */
#include <ks.h>
#include <ksmedia.h>
#include <devguid.h>
#include <devpkey.h>        /* DEVPKEY_Device_FriendlyName */
#include <ntstrsafe.h>
#include <ntintsafe.h>
#include <wdf.h>
#include <acx.h>

#include "viaduct_core.h"

#define VD_POOL_TAG 'dvaV'
#define VD_SAMPLE_RATE 48000u
/* Frame sizes of the two mono formats Windows may stream in (ADR 0005). */
#define VD_FLOAT_BYTES_PER_FRAME 4u   /* 32-bit float: the Ring's own format */
#define VD_PCM16_BYTES_PER_FRAME 2u   /* 16-bit PCM: converted in the copy */

/* Identifies Viaduct's circuits to ACX. */
DEFINE_GUID(VIADUCT_COMPONENT_GUID,
            0x6c1a1f0e, 0x3b7d, 0x4b52, 0x9a, 0x61, 0x2f, 0x0d, 0x84, 0x51, 0x7c, 0xe3);

/* Per circuit device: one per live Device. */
typedef struct _VD_CIRCUIT_DEVICE_CONTEXT {
    UINT32 DeviceId;
    UINT32 Kind;                    /* VD_KIND_MICROPHONE or VD_KIND_SPEAKER */
    vd_ring *Ring;                  /* in the core's table; only under Rundown */
    EX_RUNDOWN_REF Rundown;         /* streaming access to Ring */
    volatile LONG Deleted;          /* set once Windows starts deleting the device */
    ACXCIRCUIT Circuit;             /* created with the device, kept until it is deleted */
    BOOLEAN CircuitAdded;           /* added at the first prepare-hardware */
    UNICODE_STRING Name;
    WCHAR NameBuffer[VD_DEVICE_NAME_MAX + 1];
} VD_CIRCUIT_DEVICE_CONTEXT;
WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(VD_CIRCUIT_DEVICE_CONTEXT, VdCircuitDeviceContext)

/* The shell's resources for one live Device, kept beside the core's table. */
typedef struct _VD_SLOT {
    UINT32 DeviceId;                /* 0: unused */
    PMDL Mdl;                       /* the pinned Ring (ADR 0006) */
    WDFDEVICE CircuitDevice;        /* referenced by the slot until teardown */
} VD_SLOT;

typedef struct _VD_DRIVER_CONTEXT {
    vd_core Core;
    WDFWAITLOCK Lock;               /* serializes every core call and removal */
    WDFDEVICE Root;
    WDFDEVICE Control;
    VD_SLOT Slots[VD_MAX_DEVICES];
} VD_DRIVER_CONTEXT;
WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(VD_DRIVER_CONTEXT, VdDriverContext)

/* Per open handle on the control device: one Owner. */
typedef struct _VD_FILE_CONTEXT {
    vd_owner Owner;
} VD_FILE_CONTEXT;
WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(VD_FILE_CONTEXT, VdFileContext)

/* Per request on the control device: a Ring pinned in the caller's context. */
typedef struct _VD_REQUEST_CONTEXT {
    PMDL Mdl;
} VD_REQUEST_CONTEXT;
WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(VD_REQUEST_CONTEXT, VdRequestContext)

/* driver.c */
DRIVER_INITIALIZE DriverEntry;

/* control.c */
NTSTATUS VdControlDeviceCreate(_In_ WDFDRIVER Driver);
VOID VdUnpinRing(_In_ PMDL Mdl);

/* endpoint.c */
NTSTATUS VdCircuitDeviceAdd(_In_ VD_DRIVER_CONTEXT *Ctx, _In_ UINT32 DeviceId,
                            _In_ const vd_create_device_request *Request,
                            _Out_ WDFDEVICE *CircuitDevice);
VOID VdDeviceTeardown(_In_ VD_DRIVER_CONTEXT *Ctx, _In_ UINT32 DeviceId);

/* circuit.c */
PCUNICODE_STRING VdCircuitName(_In_ UINT32 Kind);
NTSTATUS VdCircuitCreate(_In_ WDFDEVICE Device, _Inout_ VD_CIRCUIT_DEVICE_CONTEXT *DeviceCtx);

/* interfaces.c */
NTSTATUS VdEndpointInterfacesPrepare(_In_ WDFDEVICE Device, _In_ const VD_CIRCUIT_DEVICE_CONTEXT *DeviceCtx);

/* stream.c */
EVT_ACX_CIRCUIT_CREATE_STREAM VdEvtCircuitCreateStream;

#endif /* VIADUCT_SHELL_H */
