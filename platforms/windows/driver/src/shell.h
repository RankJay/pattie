/*
 * Pattie driver shell: shared declarations.
 *
 * The shell only translates between Windows and the pure core (ADR 0002): every
 * decision lives in pattie_core.h. Layout (ADR 0010):
 *
 *   root ACX device   installed by the INF; parents the circuit devices
 *   control device    \\.\Pattie, where Owners send commands (ADR 0011 access)
 *   circuit devices   one raw child per Device, carrying one capture or render
 *                     circuit, added and removed at runtime
 *
 * Files, one concern each:
 *
 *   driver.c      DriverEntry and the root ACX device
 *   control.c     the control device, transport only: Owners, commands, Ring pinning
 *   endpoint.c    a Device's whole lifecycle: slot, core, stable ID, circuit
 *                 device (create, prepare, name, tear down) and the undo
 *   circuit.c     the circuit inside it: pins and the formats Windows may use
 *   interfaces.c  the audio interface values an INF would normally write
 *   stream.c      streaming: one copy per packet between Windows and the Ring
 *   trace.c       the driver's ETW provider (trace.h)
 *
 * Lifetime rule: everything touching a Device's Ring from the streaming timer
 * holds the circuit device's rundown protection. Removal runs it down, then
 * unpins the Ring, all under the driver lock, so no Ring memory is touched after
 * it is unpinned and no core slot is reused mid-removal. The Device's doorbell
 * event (ADR 0021) follows the same rule: set only under rundown, dereferenced
 * only after it.
 */
#ifndef PATTIE_SHELL_H
#define PATTIE_SHELL_H

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

#include "pattie_core.h"
#include "trace.h"

/* "Patt" in poolmon and !poolused: a pool tag's bytes are stored reversed. */
#define PATTIE_POOL_TAG 'ttaP'
/*
 * Create Device requests that may hold a pinned Ring at once, between the
 * caller's context and their completion: twice the Device limit. More are
 * refused at once (STATUS_DEVICE_BUSY), so no app can lock memory without bound
 * by sending many at a time (security review 2026-11-14, finding 2).
 */
#define PATTIE_MAX_PINNED_REQUESTS (2u * PATTIE_MAX_DEVICES)
/* Sample sizes of the two encodings Windows may stream in (ADR 0005, ADR 0023). */
#define PATTIE_FLOAT_BYTES_PER_SAMPLE 4u  /* 32-bit float: the Ring's own format */
#define PATTIE_PCM16_BYTES_PER_SAMPLE 2u  /* 16-bit PCM: converted in the copy */

/* Identifies Pattie's circuits to ACX. */
DEFINE_GUID(PATTIE_COMPONENT_GUID,
            0x6c1a1f0e, 0x3b7d, 0x4b52, 0x9a, 0x61, 0x2f, 0x0d, 0x84, 0x51, 0x7c, 0xe3);

/* Per circuit device: one per live Device. */
typedef struct _VD_CIRCUIT_DEVICE_CONTEXT {
    UINT32 DeviceId;
    UINT32 Kind;                    /* PATTIE_KIND_MICROPHONE or PATTIE_KIND_SPEAKER */
    pattie_ring *Ring;                  /* in the core's table; only under Rundown */
    UINT32 SampleRate;              /* the Ring's, as the core checked it at attach (ADR 0023) */
    UINT32 Channels;                /* likewise: samples in each frame */
    /*
     * The host pin's two formats at that rate and channel count (circuit.c). Kept
     * here, with the device, so they outlive the circuit whose format objects
     * were made from them.
     */
    KSDATAFORMAT_WAVEFORMATEXTENSIBLE FormatFloat;
    KSDATAFORMAT_WAVEFORMATEXTENSIBLE FormatPcm16;
    EX_RUNDOWN_REF Rundown;         /* streaming access to Ring and WakeEvent */
    PKEVENT WakeEvent;              /* the app's doorbell (ADR 0021); only under Rundown */
    volatile LONG Deleted;          /* set once Windows starts deleting the device */
    ULONG64 ChildId;                /* stable ID from kind + name; 0 once released */
    ACXCIRCUIT Circuit;             /* created with the device, kept until it is deleted */
    BOOLEAN CircuitAdded;           /* added at the first prepare-hardware */
    UNICODE_STRING Name;
    WCHAR NameBuffer[PATTIE_DEVICE_NAME_MAX + 1];
} PATTIE_CIRCUIT_DEVICE_CONTEXT;
WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(PATTIE_CIRCUIT_DEVICE_CONTEXT, PattieCircuitDeviceContext)

/* The shell's resources for one live Device, kept beside the core's table. */
typedef struct _VD_SLOT {
    UINT32 DeviceId;                /* 0: unused */
    PMDL Mdl;                       /* the pinned Ring (ADR 0006) */
    PKEVENT WakeEvent;              /* referenced doorbell event (ADR 0021) */
    WDFDEVICE CircuitDevice;        /* referenced by the slot until teardown */
} PATTIE_SLOT;

typedef struct _VD_DRIVER_CONTEXT {
    pattie_core Core;
    WDFWAITLOCK Lock;               /* serializes every core call and removal */
    WDFDEVICE Root;
    WDFDEVICE Control;
    PATTIE_SLOT Slots[PATTIE_MAX_DEVICES];
    /*
     * Stable IDs of circuit devices not yet deleted by Windows: live ones plus
     * ones still being removed (removal is asynchronous). Claimed under Lock,
     * released lock-free by the device's cleanup. 0: free. ADR 0015.
     */
    volatile LONG64 ChildIds[2 * PATTIE_MAX_DEVICES];
    volatile LONG PinnedRequests;   /* Create requests counted against PATTIE_MAX_PINNED_REQUESTS */
    /*
     * Open handles on the control device (Owners): counted from a successful
     * create to its cleanup. While any is open the root vetoes query-remove,
     * so a driver update or uninstall waits for a reboot instead of deleting
     * the control device under them (ADR 0024).
     */
    volatile LONG OpenOwners;
} PATTIE_DRIVER_CONTEXT;
WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(PATTIE_DRIVER_CONTEXT, PattieDriverContext)

/* Per open handle on the control device: one Owner. */
typedef struct _VD_FILE_CONTEXT {
    pattie_owner Owner;
} PATTIE_FILE_CONTEXT;
WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(PATTIE_FILE_CONTEXT, PattieFileContext)

/* Per request on the control device: a Ring pinned in the caller's context. */
typedef struct _VD_REQUEST_CONTEXT {
    PMDL Mdl;
    PKEVENT WakeEvent;              /* referenced in the caller's context too */
    BOOLEAN CountedPinned;          /* holds one of PATTIE_MAX_PINNED_REQUESTS until cleanup */
} PATTIE_REQUEST_CONTEXT;
WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(PATTIE_REQUEST_CONTEXT, PattieRequestContext)

/* driver.c */
DRIVER_INITIALIZE DriverEntry;

/* control.c */
NTSTATUS PattieControlDeviceCreate(_In_ WDFDRIVER Driver);
VOID PattieUnpinRing(_In_ PMDL Mdl);

/*
 * endpoint.c, each called under the driver lock.
 *
 * Create Device: the core's create, then the Device's circuit device; undone in
 * the core if that fails. On success the slot takes over *Mdl and *Wake (both set
 * to NULL); otherwise they stay the caller's to release.
 */
pattie_status PattieDeviceCreate(_In_ PATTIE_DRIVER_CONTEXT *Ctx, _In_ pattie_owner *Owner,
                         _In_reads_bytes_(InputLength) const UCHAR *Input, _In_ size_t InputLength,
                         _Inout_ PMDL *Mdl, _Inout_ PKEVENT *Wake,
                         _Out_writes_bytes_to_(ReplyCap, *ReplyLength) UCHAR *Reply, _In_ size_t ReplyCap,
                         _Out_ size_t *ReplyLength);
/* Remove Device: the core's remove, then the Device's teardown. */
pattie_status PattieDeviceRemove(_In_ PATTIE_DRIVER_CONTEXT *Ctx, _In_ pattie_owner *Owner,
                         _In_reads_bytes_(InputLength) const UCHAR *Input, _In_ size_t InputLength,
                         _Out_writes_bytes_to_(ReplyCap, *ReplyLength) UCHAR *Reply, _In_ size_t ReplyCap,
                         _Out_ size_t *ReplyLength);
/*
 * An Owner's handle closed, crash included: removes and tears down all its
 * Devices, and the core refuses the Owner's later commands, so a Create still
 * queued or in flight can't make a Device that outlives it.
 */
VOID PattieOwnerDevicesRemove(_In_ PATTIE_DRIVER_CONTEXT *Ctx, _Inout_ pattie_owner *Owner);

/* circuit.c */
PCUNICODE_STRING PattieCircuitName(_In_ UINT32 Kind);
NTSTATUS PattieCircuitCreate(_In_ WDFDEVICE Device, _Inout_ PATTIE_CIRCUIT_DEVICE_CONTEXT *DeviceCtx);

/* interfaces.c */
NTSTATUS PattieEndpointInterfacesPrepare(_In_ WDFDEVICE Device, _In_ const PATTIE_CIRCUIT_DEVICE_CONTEXT *DeviceCtx);

/* stream.c */
EVT_ACX_CIRCUIT_CREATE_STREAM PattieEvtCircuitCreateStream;

#endif /* PATTIE_SHELL_H */
