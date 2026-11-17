/*
 * A Device's whole lifecycle in the shell (ADR 0010): the slot beside the core's
 * table, the core's create and remove, the stable ID's claim, the circuit device
 * (one raw child of the root per live Device), and the undo when a step fails.
 * control.c is only the transport that calls in here; the circuit is built in
 * circuit.c and its audio interface values are written in interfaces.c. Every
 * entry point runs under the driver lock.
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

static EVT_WDF_DEVICE_PREPARE_HARDWARE EvtCircuitDevicePrepareHardware;
static EVT_WDF_OBJECT_CONTEXT_CLEANUP EvtCircuitDeviceCleanup;

/* Hardware ID; the INF matches it to install the child's null driver. */
DECLARE_CONST_UNICODE_STRING(CircuitDeviceId, L"Pattie\\Endpoint");

/* How long Create waits for an earlier child with the same stable ID to go. */
#define CHILD_ID_WAIT_MS 2000u
#define CHILD_ID_POLL_MS 10u

/*
 * A child's stable ID is the core's (pattie_stable_id, ADR 0015): the same Device
 * name comes back as the same child instance, so Windows reuses its endpoint (no
 * "2- Agent Mic", and apps remember it). The core never gives 0, which the table
 * below keeps for "free".
 *
 * Claims Id in the driver's table of children not yet deleted. Fails while a child
 * with that ID still exists, however far its removal has got: two siblings with
 * one instance ID would bugcheck PnP. Called under the driver lock.
 */
static BOOLEAN ChildIdTryClaim(DRIVER_CONTEXT *Ctx, ULONG64 Id)
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
static VOID ChildIdRelease(DRIVER_CONTEXT *Ctx, ULONG64 Id)
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
static NTSTATUS ChildIdClaim(DRIVER_CONTEXT *Ctx, ULONG64 Id)
{
    LARGE_INTEGER poll;

    poll.QuadPart = -(LONGLONG)CHILD_ID_POLL_MS * 10000; /* relative, 100 ns units */
    for (ULONG waited = 0; !ChildIdTryClaim(Ctx, Id); waited += CHILD_ID_POLL_MS) {
        if (waited >= CHILD_ID_WAIT_MS) {
            return STATUS_DEVICE_BUSY;
        }
        (void)KeDelayExecutionThread(KernelMode, FALSE, &poll);
    }
    return STATUS_SUCCESS;
}

/*
 * Gives a Device the core just accepted its circuit device. On success the child
 * holds the stable ID until Windows deletes it; on failure nothing is left.
 */
static NTSTATUS CircuitDeviceAdd(DRIVER_CONTEXT *Ctx, UINT32 DeviceId,
                                 const pattie_device_info *Info, PKEVENT WakeEvent,
                                 WDFDEVICE *CircuitDevice)
{
    PWDFDEVICE_INIT init;
    ACX_DEVICEINIT_CONFIG acxInitConfig;
    ACX_DEVICE_CONFIG acxConfig;
    WDF_PNPPOWER_EVENT_CALLBACKS pnpCallbacks;
    WDF_OBJECT_ATTRIBUTES attributes;
    CIRCUIT_DEVICE_CONTEXT *deviceCtx;
    DECLARE_UNICODE_STRING_SIZE(instanceId, 24); /* kind letter + 16 hex digits */
    UNICODE_STRING name;
    ULONG64 childId;
    WDFDEVICE device;
    NTSTATUS status;

    *CircuitDevice = NULL;

    name.Buffer = (PWCH)Info->name;
    name.Length = (USHORT)(Info->name_len * sizeof(WCHAR));
    name.MaximumLength = name.Length;

    /* Claimed before the child exists; released by its cleanup, or below on failure. */
    childId = Info->stable_id;
    status = ChildIdClaim(Ctx, childId);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    init = WdfPdoInitAllocate(Ctx->Root);
    if (init == NULL) {
        ChildIdRelease(Ctx, childId);
        return STATUS_INSUFFICIENT_RESOURCES;
    }

    status = RtlUnicodeStringPrintf(&instanceId, L"%c%016I64x",
                                    Info->kind == PATTIE_KIND_MICROPHONE ? L'M' : L'S', childId);
    if (NT_SUCCESS(status)) {
        status = WdfPdoInitAssignDeviceID(init, &CircuitDeviceId);
    }
    if (NT_SUCCESS(status)) {
        status = WdfPdoInitAddHardwareID(init, &CircuitDeviceId);
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
        ChildIdRelease(Ctx, childId);
        return status;
    }

    WDF_PNPPOWER_EVENT_CALLBACKS_INIT(&pnpCallbacks);
    pnpCallbacks.EvtDevicePrepareHardware = EvtCircuitDevicePrepareHardware;
    WdfDeviceInitSetPnpPowerEventCallbacks(init, &pnpCallbacks);

    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, CIRCUIT_DEVICE_CONTEXT);
    attributes.EvtCleanupCallback = EvtCircuitDeviceCleanup;
    /*
     * Default execution level, unlike the XU sample's passive one: our streams run
     * a DISPATCH-level, automatically serialized timer under this device, which
     * WDF refuses beneath a passive parent. Microsoft's AudioCodec sample, the one
     * with a stream timer, keeps the default too.
     */
    status = WdfDeviceCreate(&init, &attributes, &device);
    if (!NT_SUCCESS(status)) {
        WdfDeviceInitFree(init);
        ChildIdRelease(Ctx, childId);
        return status;
    }

    deviceCtx = CircuitDeviceContext(device);
    deviceCtx->ChildId = childId; /* from here on, the device's cleanup releases it */
    deviceCtx->DeviceId = DeviceId;
    deviceCtx->Kind = Info->kind;
    deviceCtx->Ring = pattie_core_device_ring(&Ctx->Core, DeviceId);
    /* Read once: the core's own copies, checked when it attached the Ring. */
    deviceCtx->SampleRate = pattie_ring_sample_rate(deviceCtx->Ring);
    deviceCtx->Channels = pattie_ring_channels(deviceCtx->Ring);
    deviceCtx->WakeEvent = WakeEvent; /* the slot holds the reference */
    ExInitializeRundownProtection(&deviceCtx->Rundown);
    RtlCopyMemory(deviceCtx->NameBuffer, Info->name, name.Length);
    deviceCtx->Name.Buffer = deviceCtx->NameBuffer;
    deviceCtx->Name.Length = name.Length;
    deviceCtx->Name.MaximumLength = sizeof deviceCtx->NameBuffer;

    ACX_DEVICE_CONFIG_INIT(&acxConfig);
    status = AcxDeviceInitialize(device, &acxConfig);
    if (NT_SUCCESS(status)) {
        /* Created with the device, before ACX sees it (Microsoft's pattern). */
        status = CircuitCreate(device, deviceCtx);
    }
    if (NT_SUCCESS(status)) {
        status = AcxDeviceAddCircuitDevice(Ctx->Root, device);
    }
    if (!NT_SUCCESS(status)) {
        WdfObjectDelete(device);
        return status;
    }

    /* With CircuitPrepared, times how long Windows takes to start the child. */
    TraceLoggingWrite(TraceProvider, "CircuitDeviceAdded", TraceLoggingLevel(TRACE_LEVEL_INFO),
                      TraceLoggingUInt32(DeviceId, "DeviceId"),
                      TraceLoggingUInt32(Info->kind, "Kind"));
    *CircuitDevice = device;
    return STATUS_SUCCESS;
}

/*
 * The core already dropped the Device. Called under the driver lock, so the core
 * can't reuse its slot until the Ring is safely unpinned (ADR 0010).
 */
static VOID DeviceTeardown(DRIVER_CONTEXT *Ctx, UINT32 DeviceId)
{
    for (ULONG i = 0; i < PATTIE_MAX_DEVICES; i++) {
        SLOT *slot = &Ctx->Slots[i];

        if (slot->DeviceId != DeviceId) {
            continue;
        }
        if (slot->CircuitDevice != NULL) {
            /* 1. No new stream access; wait out any in-flight timer callback. */
            ExWaitForRundownProtectionRelease(&CircuitDeviceContext(slot->CircuitDevice)->Rundown);
        }
        /* 2. Unpin before the app's process can finish exiting, and let go of its
         *    doorbell: no timer callback can set it any more. */
        if (slot->Mdl != NULL) {
            UnpinRing(slot->Mdl);
        }
        if (slot->WakeEvent != NULL) {
            ObDereferenceObject(slot->WakeEvent);
        }
        /* 3. Ask Windows to remove the endpoint (asynchronous), unless it already did. */
        if (slot->CircuitDevice != NULL) {
            if (Ctx->Root != NULL &&
                InterlockedCompareExchange(&CircuitDeviceContext(slot->CircuitDevice)->Deleted, 0,
                                           0) == 0) {
                (void)AcxDeviceRemoveCircuitDevice(Ctx->Root, slot->CircuitDevice);
            }
            WdfObjectDereference(slot->CircuitDevice); /* taken when the slot was filled */
        }
        RtlZeroMemory(slot, sizeof *slot);
        TraceLoggingWrite(TraceProvider, "CircuitDeviceRemoved",
                          TraceLoggingLevel(TRACE_LEVEL_INFO),
                          TraceLoggingUInt32(DeviceId, "DeviceId"));
        return;
    }
}

_Use_decl_annotations_ pattie_status DeviceCreate(DRIVER_CONTEXT *Ctx, pattie_owner *Owner,
                                                  const UCHAR *Input, size_t InputLength, PMDL *Mdl,
                                                  PKEVENT *Wake, UCHAR *Reply, size_t ReplyCap,
                                                  size_t *ReplyLength)
{
    pattie_create_device_reply created;
    pattie_device_info info;
    void *ringMemory = NULL;
    size_t ringLength = 0;
    WDFDEVICE circuitDevice;
    SLOT *slot = NULL;
    pattie_status result;

    *ReplyLength = 0;
    if (Ctx->Root == NULL) {
        return PATTIE_ERR_SYSTEM; /* the root device is going away */
    }
    /* Opaque to the core (ADR 0021): the shell alone needs a doorbell it can set. */
    if (*Wake == NULL) {
        return PATTIE_ERR_BAD_REQUEST;
    }
    /* Claim the shell's slot first, so an accepted Device always has one. Slots
     * mirror the core's PATTIE_MAX_DEVICES, so this only fails when the core would. */
    for (ULONG i = 0; i < PATTIE_MAX_DEVICES && slot == NULL; i++) {
        if (Ctx->Slots[i].DeviceId == 0) {
            slot = &Ctx->Slots[i];
        }
    }
    if (slot == NULL) {
        return PATTIE_ERR_TOO_MANY_DEVICES;
    }
    if (*Mdl != NULL) {
        ringMemory = MmGetSystemAddressForMdlSafe(*Mdl, NormalPagePriority | MdlMappingNoExecute);
        ringLength = MmGetMdlByteCount(*Mdl);
    }

    result = pattie_core_command(&Ctx->Core, Owner, PATTIE_CMD_CREATE_DEVICE, Input, InputLength,
                                 ringMemory, ringLength, Reply, ReplyCap, ReplyLength);
    if (result != PATTIE_OK) {
        return result;
    }

    /* What the core accepted, kind, name and stable ID: the request isn't read again. */
    RtlCopyMemory(&created, Reply, sizeof created);
    if (!pattie_core_device_info(&Ctx->Core, created.device_id, &info) ||
        !NT_SUCCESS(CircuitDeviceAdd(Ctx, created.device_id, &info, *Wake, &circuitDevice))) {
        /* Undo through the core's own Remove Device, then report the failure. */
        pattie_remove_device_request undo = {created.device_id};
        size_t ignored;
        (void)pattie_core_command(&Ctx->Core, Owner, PATTIE_CMD_REMOVE_DEVICE, &undo, sizeof undo,
                                  NULL, 0, NULL, 0, &ignored);
        *ReplyLength = 0;
        return PATTIE_ERR_SYSTEM;
    }

    /* The slot's reference keeps the circuit device's context (and its rundown)
     * valid until teardown, however Windows deletes the device meanwhile. */
    WdfObjectReference(circuitDevice);
    slot->DeviceId = created.device_id;
    slot->Mdl = *Mdl;
    slot->WakeEvent = *Wake;
    slot->CircuitDevice = circuitDevice;
    *Mdl = NULL;  /* the slot owns the pinned Ring now */
    *Wake = NULL; /* and the doorbell's reference */
    return PATTIE_OK;
}

_Use_decl_annotations_ pattie_status DeviceRemove(DRIVER_CONTEXT *Ctx, pattie_owner *Owner,
                                                  const UCHAR *Input, size_t InputLength,
                                                  UCHAR *Reply, size_t ReplyCap,
                                                  size_t *ReplyLength)
{
    pattie_status result;

    *ReplyLength = 0;
    result = pattie_core_command(&Ctx->Core, Owner, PATTIE_CMD_REMOVE_DEVICE, Input, InputLength,
                                 NULL, 0, Reply, ReplyCap, ReplyLength);

    if (result == PATTIE_OK) {
        pattie_remove_device_request removed;

        RtlCopyMemory(&removed, Input, sizeof removed); /* the core validated it */
        DeviceTeardown(Ctx, removed.device_id);
    }
    return result;
}

_Use_decl_annotations_ VOID OwnerDevicesRemove(DRIVER_CONTEXT *Ctx, pattie_owner *Owner)
{
    uint32_t removed[PATTIE_MAX_DEVICES];
    size_t count = pattie_core_owner_closed(&Ctx->Core, Owner, removed);

    for (size_t i = 0; i < count; i++) {
        DeviceTeardown(Ctx, removed[i]);
    }
}

/*
 * Windows is deleting the circuit device. Takes no lock (teardown may hold the
 * driver lock while asking ACX to remove this device): it records the fact, and
 * frees the device's stable ID so a Device of the same name can be created again.
 * The slot's reference keeps this context valid until teardown releases it.
 */
static VOID EvtCircuitDeviceCleanup(WDFOBJECT Object)
{
    CIRCUIT_DEVICE_CONTEXT *deviceCtx = CircuitDeviceContext((WDFDEVICE)Object);

    InterlockedExchange(&deviceCtx->Deleted, 1);
    if (deviceCtx->ChildId != 0) {
        ChildIdRelease(DriverContext(WdfGetDriver()), deviceCtx->ChildId);
        deviceCtx->ChildId = 0;
    }
}

/*
 * Names the child after its Device. Once Windows installs the child's null driver,
 * the INF's description replaces the device text the PDO was created with, and the
 * endpoint shows "Pattie Audio Endpoint" (VM, 2026-11-07); the endpoint builder
 * prefers the devnode's FriendlyName. Cosmetic, so a failure is ignored: it must
 * never keep the endpoint from starting.
 */
static VOID AssignFriendlyName(WDFDEVICE Device, const CIRCUIT_DEVICE_CONTEXT *DeviceCtx)
{
    WDF_DEVICE_PROPERTY_DATA property;

    WDF_DEVICE_PROPERTY_DATA_INIT(&property, &DEVPKEY_Device_FriendlyName);
    property.Lcid = LOCALE_NEUTRAL;
    /* NameBuffer is zero-filled past the name, so it is NUL-terminated. */
    (void)WdfDeviceAssignProperty(Device, &property, DEVPROP_TYPE_STRING,
                                  DeviceCtx->Name.Length + sizeof(WCHAR),
                                  (PVOID)DeviceCtx->NameBuffer);
}

static NTSTATUS EvtCircuitDevicePrepareHardware(WDFDEVICE Device, WDFCMRESLIST Resources,
                                                WDFCMRESLIST ResourcesTranslated)
{
    CIRCUIT_DEVICE_CONTEXT *deviceCtx = CircuitDeviceContext(Device);
    NTSTATUS status;

    UNREFERENCED_PARAMETER(Resources);
    UNREFERENCED_PARAMETER(ResourcesTranslated);

    /* Every prepare, so the name survives the restart that installing the child causes. */
    AssignFriendlyName(Device, deviceCtx);

    if (deviceCtx->CircuitAdded) {
        return STATUS_SUCCESS; /* rebalance or restart: keep the circuit */
    }
    status = EndpointInterfacesPrepare(Device, deviceCtx);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    status = AcxDeviceAddCircuit(Device, deviceCtx->Circuit);
    if (NT_SUCCESS(status)) {
        deviceCtx->CircuitAdded = TRUE;
        TraceLoggingWrite(TraceProvider, "CircuitPrepared", TraceLoggingLevel(TRACE_LEVEL_INFO),
                          TraceLoggingUInt32(deviceCtx->DeviceId, "DeviceId"));
    }
    return status;
}
