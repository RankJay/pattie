/*
 * Circuit devices: one raw child of the root per live Device (ADR 0010). This
 * file owns the child's lifecycle; its circuit is built in circuit.c and its
 * audio interface values are written in interfaces.c.
 *
 * Follows Microsoft's runtime circuit-device pattern (SdcaVad sample, the XU
 * driver's dynamic capture devices): allocate a child with WdfPdoInitAllocate,
 * create it and its one circuit, hand it to ACX with AcxDeviceAddCircuitDevice,
 * and add the circuit the first time the child prepares its hardware. Later
 * prepare-hardware calls (a rebalance, or the restart that installing the child's
 * null driver causes) keep it, and there is no release-hardware callback: the
 * circuit leaves only with the whole child, through AcxDeviceRemoveCircuitDevice,
 * which completes asynchronously. Removing the circuit on release and re-adding
 * it crashed inside ACX (VM, 2026-11-07).
 */
#include "shell.h"

static EVT_WDF_DEVICE_PREPARE_HARDWARE VdEvtCircuitDevicePrepareHardware;
static EVT_WDF_OBJECT_CONTEXT_CLEANUP VdEvtCircuitDeviceCleanup;

/* Hardware ID; the INF matches it to install the child's null driver. */
DECLARE_CONST_UNICODE_STRING(VdCircuitDeviceId, L"Viaduct\\Endpoint");

/* How long Create waits for an earlier child with the same stable ID to go. */
#define VD_CHILD_ID_WAIT_MS 2000u
#define VD_CHILD_ID_POLL_MS 10u

/*
 * A child's stable ID (ADR 0015): a 64-bit FNV-1a hash of the Device's kind and
 * name (case-sensitive, like names: ADR 0004). The same Device name comes back as
 * the same child instance, so Windows reuses its endpoint (no "2- Agent Mic", and
 * apps remember it). 0 is reserved for "free".
 */
static ULONG64 VdChildId(UINT32 Kind, const WCHAR *Name, USHORT NameBytes)
{
    const UCHAR *bytes = (const UCHAR *)Name;
    ULONG64 hash = 0xcbf29ce484222325ull;

    hash = (hash ^ (UCHAR)Kind) * 0x100000001b3ull;
    for (USHORT i = 0; i < NameBytes; i++) {
        hash = (hash ^ bytes[i]) * 0x100000001b3ull;
    }
    return hash == 0 ? 1 : hash;
}

/*
 * Claims Id in the driver's table of children not yet deleted. Fails while a child
 * with that ID still exists, however far its removal has got: two siblings with
 * one instance ID would bugcheck PnP. Called under the driver lock.
 */
static BOOLEAN VdChildIdTryClaim(VD_DRIVER_CONTEXT *Ctx, ULONG64 Id)
{
    volatile LONG64 *free = NULL;

    for (ULONG i = 0; i < ARRAYSIZE(Ctx->ChildIds); i++) {
        LONG64 held = InterlockedCompareExchange64(&Ctx->ChildIds[i], 0, 0);

        if ((ULONG64)held == Id) {
            return FALSE; /* that child is still live or being removed */
        }
        if (held == 0 && free == NULL) {
            free = &Ctx->ChildIds[i];
        }
    }
    /* Only cleanup changes entries outside the lock, and only to 0, so free stays free. */
    return free != NULL && InterlockedCompareExchange64(free, (LONG64)Id, 0) == 0;
}

/* Releases Id once its child is gone (or was never created). Lock-free. */
static VOID VdChildIdRelease(VD_DRIVER_CONTEXT *Ctx, ULONG64 Id)
{
    for (ULONG i = 0; i < ARRAYSIZE(Ctx->ChildIds); i++) {
        if (InterlockedCompareExchange64(&Ctx->ChildIds[i], 0, (LONG64)Id) == (LONG64)Id) {
            return;
        }
    }
}

/*
 * Claims Id, waiting (bounded) while an earlier child with the same ID is still
 * being removed: an app that drops "Agent Mic" and creates it again at once gets
 * the same endpoint back a moment later instead of a failure.
 */
static NTSTATUS VdChildIdClaim(VD_DRIVER_CONTEXT *Ctx, ULONG64 Id)
{
    LARGE_INTEGER poll;

    poll.QuadPart = -(LONGLONG)VD_CHILD_ID_POLL_MS * 10000; /* relative, 100 ns units */
    for (ULONG waited = 0; !VdChildIdTryClaim(Ctx, Id); waited += VD_CHILD_ID_POLL_MS) {
        if (waited >= VD_CHILD_ID_WAIT_MS) {
            return STATUS_DEVICE_BUSY;
        }
        (void)KeDelayExecutionThread(KernelMode, FALSE, &poll);
    }
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS VdCircuitDeviceAdd(VD_DRIVER_CONTEXT *Ctx, UINT32 DeviceId,
                            const vd_create_device_request *Request, WDFDEVICE *CircuitDevice)
{
    PWDFDEVICE_INIT init;
    ACX_DEVICEINIT_CONFIG acxInitConfig;
    ACX_DEVICE_CONFIG acxConfig;
    WDF_PNPPOWER_EVENT_CALLBACKS pnpCallbacks;
    WDF_OBJECT_ATTRIBUTES attributes;
    VD_CIRCUIT_DEVICE_CONTEXT *deviceCtx;
    DECLARE_UNICODE_STRING_SIZE(instanceId, 24); /* kind letter + 16 hex digits */
    UNICODE_STRING name;
    ULONG64 childId;
    WDFDEVICE device;
    NTSTATUS status;

    *CircuitDevice = NULL;

    name.Buffer = (PWCH)Request->name;
    name.Length = (USHORT)(Request->name_len * sizeof(WCHAR));
    name.MaximumLength = name.Length;

    /* Claimed before the child exists; released by its cleanup, or below on failure. */
    childId = VdChildId(Request->kind, name.Buffer, name.Length);
    status = VdChildIdClaim(Ctx, childId);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    init = WdfPdoInitAllocate(Ctx->Root);
    if (init == NULL) {
        VdChildIdRelease(Ctx, childId);
        return STATUS_INSUFFICIENT_RESOURCES;
    }

    status = RtlUnicodeStringPrintf(&instanceId, L"%c%016I64x",
                                    Request->kind == VD_KIND_MICROPHONE ? L'M' : L'S', childId);
    if (NT_SUCCESS(status)) {
        status = WdfPdoInitAssignDeviceID(init, &VdCircuitDeviceId);
    }
    if (NT_SUCCESS(status)) {
        status = WdfPdoInitAddHardwareID(init, &VdCircuitDeviceId);
    }
    if (NT_SUCCESS(status)) {
        status = WdfPdoInitAssignInstanceID(init, &instanceId);
    }
    if (NT_SUCCESS(status)) {
        status = WdfPdoInitAddDeviceText(init, &name, &name, 0x409);
    }
    if (NT_SUCCESS(status)) {
        WdfPdoInitSetDefaultLocale(init, 0x409);
        status = WdfPdoInitAssignRawDevice(init, &GUID_DEVCLASS_MEDIA);
    }
    if (NT_SUCCESS(status)) {
        ACX_DEVICEINIT_CONFIG_INIT(&acxInitConfig);
        acxInitConfig.Flags |= AcxDeviceInitConfigRawDevice;
        status = AcxDeviceInitInitialize(init, &acxInitConfig);
    }
    if (!NT_SUCCESS(status)) {
        WdfDeviceInitFree(init);
        VdChildIdRelease(Ctx, childId);
        return status;
    }

    WDF_PNPPOWER_EVENT_CALLBACKS_INIT(&pnpCallbacks);
    pnpCallbacks.EvtDevicePrepareHardware = VdEvtCircuitDevicePrepareHardware;
    WdfDeviceInitSetPnpPowerEventCallbacks(init, &pnpCallbacks);

    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, VD_CIRCUIT_DEVICE_CONTEXT);
    attributes.EvtCleanupCallback = VdEvtCircuitDeviceCleanup;
    /*
     * Default execution level, unlike the XU sample's passive one: our streams run
     * a DISPATCH-level, automatically serialized timer under this device, which
     * WDF refuses beneath a passive parent. Microsoft's AudioCodec sample, the one
     * with a stream timer, keeps the default too.
     */
    status = WdfDeviceCreate(&init, &attributes, &device);
    if (!NT_SUCCESS(status)) {
        WdfDeviceInitFree(init);
        VdChildIdRelease(Ctx, childId);
        return status;
    }

    deviceCtx = VdCircuitDeviceContext(device);
    deviceCtx->ChildId = childId; /* from here on, the device's cleanup releases it */
    deviceCtx->DeviceId = DeviceId;
    deviceCtx->Kind = Request->kind;
    deviceCtx->Ring = vd_core_device_ring(&Ctx->Core, DeviceId);
    ExInitializeRundownProtection(&deviceCtx->Rundown);
    RtlCopyMemory(deviceCtx->NameBuffer, Request->name, name.Length);
    deviceCtx->Name.Buffer = deviceCtx->NameBuffer;
    deviceCtx->Name.Length = name.Length;
    deviceCtx->Name.MaximumLength = sizeof deviceCtx->NameBuffer;

    ACX_DEVICE_CONFIG_INIT(&acxConfig);
    status = AcxDeviceInitialize(device, &acxConfig);
    if (NT_SUCCESS(status)) {
        /* Created with the device, before ACX sees it (Microsoft's pattern). */
        status = VdCircuitCreate(device, deviceCtx);
    }
    if (NT_SUCCESS(status)) {
        status = AcxDeviceAddCircuitDevice(Ctx->Root, device);
    }
    if (!NT_SUCCESS(status)) {
        WdfObjectDelete(device);
        return status;
    }

    /* With CircuitPrepared, times how long Windows takes to start the child. */
    TraceLoggingWrite(VdTraceProvider, "CircuitDeviceAdded",
                      TraceLoggingLevel(VD_TRACE_LEVEL_INFO),
                      TraceLoggingUInt32(DeviceId, "DeviceId"),
                      TraceLoggingUInt32(Request->kind, "Kind"));
    *CircuitDevice = device;
    return STATUS_SUCCESS;
}

/*
 * The core already dropped the Device. Called under the driver lock, so the core
 * can't reuse its slot until the Ring is safely unpinned (ADR 0010).
 */
_Use_decl_annotations_
VOID VdDeviceTeardown(VD_DRIVER_CONTEXT *Ctx, UINT32 DeviceId)
{
    for (ULONG i = 0; i < VD_MAX_DEVICES; i++) {
        VD_SLOT *slot = &Ctx->Slots[i];

        if (slot->DeviceId != DeviceId) {
            continue;
        }
        if (slot->CircuitDevice != NULL) {
            /* 1. No new stream access; wait out any in-flight timer callback. */
            ExWaitForRundownProtectionRelease(&VdCircuitDeviceContext(slot->CircuitDevice)->Rundown);
        }
        /* 2. Unpin before the app's process can finish exiting. */
        if (slot->Mdl != NULL) {
            VdUnpinRing(slot->Mdl);
        }
        /* 3. Ask Windows to remove the endpoint (asynchronous), unless it already did. */
        if (slot->CircuitDevice != NULL) {
            if (Ctx->Root != NULL &&
                InterlockedCompareExchange(&VdCircuitDeviceContext(slot->CircuitDevice)->Deleted, 0, 0) == 0) {
                (void)AcxDeviceRemoveCircuitDevice(Ctx->Root, slot->CircuitDevice);
            }
            WdfObjectDereference(slot->CircuitDevice); /* taken when the slot was filled */
        }
        RtlZeroMemory(slot, sizeof *slot);
        TraceLoggingWrite(VdTraceProvider, "CircuitDeviceRemoved",
                          TraceLoggingLevel(VD_TRACE_LEVEL_INFO),
                          TraceLoggingUInt32(DeviceId, "DeviceId"));
        return;
    }
}

/*
 * Windows is deleting the circuit device. Takes no lock (teardown may hold the
 * driver lock while asking ACX to remove this device): it records the fact, and
 * frees the device's stable ID so a Device of the same name can be created again.
 * The slot's reference keeps this context valid until teardown releases it.
 */
static VOID VdEvtCircuitDeviceCleanup(WDFOBJECT Object)
{
    VD_CIRCUIT_DEVICE_CONTEXT *deviceCtx = VdCircuitDeviceContext((WDFDEVICE)Object);

    InterlockedExchange(&deviceCtx->Deleted, 1);
    if (deviceCtx->ChildId != 0) {
        VdChildIdRelease(VdDriverContext(WdfGetDriver()), deviceCtx->ChildId);
        deviceCtx->ChildId = 0;
    }
}

/*
 * Names the child after its Device. Once Windows installs the child's null driver,
 * the INF's description replaces the device text the PDO was created with, and the
 * endpoint shows "Viaduct Audio Endpoint" (VM, 2026-11-07); the endpoint builder
 * prefers the devnode's FriendlyName. Cosmetic, so a failure is ignored: it must
 * never keep the endpoint from starting.
 */
static VOID VdAssignFriendlyName(WDFDEVICE Device, const VD_CIRCUIT_DEVICE_CONTEXT *DeviceCtx)
{
    WDF_DEVICE_PROPERTY_DATA property;

    WDF_DEVICE_PROPERTY_DATA_INIT(&property, &DEVPKEY_Device_FriendlyName);
    property.Lcid = LOCALE_NEUTRAL;
    /* NameBuffer is zero-filled past the name, so it is NUL-terminated. */
    (void)WdfDeviceAssignProperty(Device, &property, DEVPROP_TYPE_STRING,
                                  DeviceCtx->Name.Length + sizeof(WCHAR), (PVOID)DeviceCtx->NameBuffer);
}

static NTSTATUS VdEvtCircuitDevicePrepareHardware(WDFDEVICE Device, WDFCMRESLIST Resources,
                                                  WDFCMRESLIST ResourcesTranslated)
{
    VD_CIRCUIT_DEVICE_CONTEXT *deviceCtx = VdCircuitDeviceContext(Device);
    NTSTATUS status;

    UNREFERENCED_PARAMETER(Resources);
    UNREFERENCED_PARAMETER(ResourcesTranslated);

    /* Every prepare, so the name survives the restart that installing the child causes. */
    VdAssignFriendlyName(Device, deviceCtx);

    if (deviceCtx->CircuitAdded) {
        return STATUS_SUCCESS; /* rebalance or restart: keep the circuit */
    }
    status = VdEndpointInterfacesPrepare(Device, deviceCtx);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    status = AcxDeviceAddCircuit(Device, deviceCtx->Circuit);
    if (NT_SUCCESS(status)) {
        deviceCtx->CircuitAdded = TRUE;
        TraceLoggingWrite(VdTraceProvider, "CircuitPrepared",
                          TraceLoggingLevel(VD_TRACE_LEVEL_INFO),
                          TraceLoggingUInt32(deviceCtx->DeviceId, "DeviceId"));
    }
    return status;
}
