/*
 * The control device: where Owners send commands (ADR 0010, ADR 0011).
 *
 * One open handle is one Owner. Create Device's Ring is pinned while still in
 * the caller's own context, because its address only means something there
 * (ADR 0006, ADR 0009). Every command then runs through the core under the
 * driver lock, and the reply goes back in the envelope from the ABI header.
 */
#include "shell.h"

static EVT_WDF_DEVICE_FILE_CREATE VdEvtFileCreate;
static EVT_WDF_FILE_CLEANUP VdEvtFileCleanup;
static EVT_WDF_IO_IN_CALLER_CONTEXT VdEvtIoInCallerContext;
static EVT_WDF_IO_QUEUE_IO_DEVICE_CONTROL VdEvtIoDeviceControl;
static EVT_WDF_OBJECT_CONTEXT_CLEANUP VdEvtRequestCleanup;

/* SYSTEM and Administrators: full access. Interactive users: read/write. Nothing else. */
DECLARE_CONST_UNICODE_STRING(VdControlSddl, L"D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)");
DECLARE_CONST_UNICODE_STRING(VdControlName, L"\\Device\\Viaduct");
DECLARE_CONST_UNICODE_STRING(VdControlLink, L"\\DosDevices\\Viaduct");

_Use_decl_annotations_
NTSTATUS VdControlDeviceCreate(WDFDRIVER Driver)
{
    VD_DRIVER_CONTEXT *ctx = VdDriverContext(Driver);
    PWDFDEVICE_INIT init;
    WDF_FILEOBJECT_CONFIG fileConfig;
    WDF_OBJECT_ATTRIBUTES fileAttributes;
    WDF_OBJECT_ATTRIBUTES requestAttributes;
    WDF_OBJECT_ATTRIBUTES deviceAttributes;
    WDF_IO_QUEUE_CONFIG queueConfig;
    WDFDEVICE control;
    NTSTATUS status;

    init = WdfControlDeviceInitAllocate(Driver, &VdControlSddl);
    if (init == NULL) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    WdfDeviceInitSetExclusive(init, FALSE);
    WdfDeviceInitSetIoType(init, WdfDeviceIoBuffered);
    status = WdfDeviceInitAssignName(init, &VdControlName);
    if (!NT_SUCCESS(status)) {
        WdfDeviceInitFree(init);
        return status;
    }

    WDF_FILEOBJECT_CONFIG_INIT(&fileConfig, VdEvtFileCreate, WDF_NO_EVENT_CALLBACK, VdEvtFileCleanup);
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&fileAttributes, VD_FILE_CONTEXT);
    WdfDeviceInitSetFileObjectConfig(init, &fileConfig, &fileAttributes);

    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&requestAttributes, VD_REQUEST_CONTEXT);
    requestAttributes.EvtCleanupCallback = VdEvtRequestCleanup;
    WdfDeviceInitSetRequestAttributes(init, &requestAttributes);
    WdfDeviceInitSetIoInCallerContextCallback(init, VdEvtIoInCallerContext);

    WDF_OBJECT_ATTRIBUTES_INIT(&deviceAttributes);
    deviceAttributes.ExecutionLevel = WdfExecutionLevelPassive; /* waitlock, MDL work */
    status = WdfDeviceCreate(&init, &deviceAttributes, &control);
    if (!NT_SUCCESS(status)) {
        WdfDeviceInitFree(init);
        return status;
    }

    status = WdfDeviceCreateSymbolicLink(control, &VdControlLink);
    if (!NT_SUCCESS(status)) {
        WdfObjectDelete(control);
        return status;
    }

    /* Sequential: one command at a time reaches the core (ADR 0002). */
    WDF_IO_QUEUE_CONFIG_INIT_DEFAULT_QUEUE(&queueConfig, WdfIoQueueDispatchSequential);
    queueConfig.EvtIoDeviceControl = VdEvtIoDeviceControl;
    status = WdfIoQueueCreate(control, &queueConfig, WDF_NO_OBJECT_ATTRIBUTES, WDF_NO_HANDLE);
    if (!NT_SUCCESS(status)) {
        WdfObjectDelete(control);
        return status;
    }

    WdfControlFinishInitializing(control);
    ctx->Control = control;
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
VOID VdUnpinRing(PMDL Mdl)
{
    MmUnlockPages(Mdl);
    IoFreeMdl(Mdl);
}

static VOID VdEvtFileCreate(WDFDEVICE Device, WDFREQUEST Request, WDFFILEOBJECT FileObject)
{
    UNREFERENCED_PARAMETER(Device);
    vd_owner_init(&VdFileContext(FileObject)->Owner);
    WdfRequestComplete(Request, STATUS_SUCCESS);
}

/* The Owner's last handle closed, crash included: remove all its Devices (ADR 0005). */
static VOID VdEvtFileCleanup(WDFFILEOBJECT FileObject)
{
    VD_DRIVER_CONTEXT *ctx = VdDriverContext(WdfGetDriver());
    uint32_t removed[VD_MAX_DEVICES];
    size_t count;

    WdfWaitLockAcquire(ctx->Lock, NULL);
    count = vd_core_owner_closed(&ctx->Core, &VdFileContext(FileObject)->Owner, removed);
    for (size_t i = 0; i < count; i++) {
        VdDeviceTeardown(ctx, removed[i]);
    }
    WdfWaitLockRelease(ctx->Lock);
}

/* Pins the Ring of a Create Device request; anything that can't be pinned reaches
 * the core as "no Ring", which the core rejects (ADR 0009). */
static PMDL VdTryPinRing(WDFREQUEST Request)
{
    vd_create_device_request *request;
    size_t length;
    PMDL mdl;

    if (!NT_SUCCESS(WdfRequestRetrieveInputBuffer(Request, sizeof *request, (PVOID *)&request, &length))) {
        return NULL;
    }
    if (request->ring_address == 0 || request->ring_bytes == 0 ||
        request->ring_bytes > VD_RING_MAX_BYTES) {
        return NULL;
    }
    mdl = IoAllocateMdl((PVOID)(ULONG_PTR)request->ring_address, (ULONG)request->ring_bytes,
                        FALSE, FALSE, NULL);
    if (mdl == NULL) {
        return NULL;
    }
    __try {
        MmProbeAndLockPages(mdl, UserMode, IoWriteAccess);
    } __except (EXCEPTION_EXECUTE_HANDLER) {
        IoFreeMdl(mdl);
        return NULL;
    }
    return mdl;
}

static VOID VdEvtIoInCallerContext(WDFDEVICE Device, WDFREQUEST Request)
{
    WDF_REQUEST_PARAMETERS params;
    NTSTATUS status;

    WDF_REQUEST_PARAMETERS_INIT(&params);
    WdfRequestGetParameters(Request, &params);
    if (params.Type == WdfRequestTypeDeviceControl &&
        params.Parameters.DeviceIoControl.IoControlCode == VD_IOCTL(VD_CMD_CREATE_DEVICE)) {
        VdRequestContext(Request)->Mdl = VdTryPinRing(Request);
    }

    status = WdfDeviceEnqueueRequest(Device, Request);
    if (!NT_SUCCESS(status)) {
        WdfRequestComplete(Request, status); /* request cleanup unpins */
    }
}

static VOID VdEvtRequestCleanup(WDFOBJECT Object)
{
    VD_REQUEST_CONTEXT *requestCtx = VdRequestContext((WDFREQUEST)Object);

    if (requestCtx->Mdl != NULL) {
        VdUnpinRing(requestCtx->Mdl);
        requestCtx->Mdl = NULL;
    }
}

static BOOLEAN VdCommandFromIoctl(ULONG IoControlCode, uint32_t *Command)
{
    static const uint32_t commands[] = {
        VD_CMD_HELLO, VD_CMD_CREATE_DEVICE, VD_CMD_REMOVE_DEVICE,
    };

    for (ULONG i = 0; i < ARRAYSIZE(commands); i++) {
        if (IoControlCode == VD_IOCTL(commands[i])) {
            *Command = commands[i];
            return TRUE;
        }
    }
    return FALSE;
}

/* Runs Create Device and, if the core accepted it, gives it a circuit device. */
static vd_status VdCreateDevice(VD_DRIVER_CONTEXT *ctx, vd_owner *owner, const UCHAR *input,
                                size_t inputLength, PMDL *mdl, UCHAR *reply, size_t replyCap,
                                size_t *replyLength)
{
    vd_create_device_request request;
    vd_create_device_reply created;
    void *ringMemory = NULL;
    size_t ringLength = 0;
    WDFDEVICE circuitDevice;
    VD_SLOT *slot = NULL;
    vd_status result;

    if (ctx->Root == NULL) {
        return VD_ERR_SYSTEM; /* the root device is going away */
    }
    /* Claim the shell's slot first, so an accepted Device always has one. Slots
     * mirror the core's VD_MAX_DEVICES, so this only fails when the core would. */
    for (ULONG i = 0; i < VD_MAX_DEVICES && slot == NULL; i++) {
        if (ctx->Slots[i].DeviceId == 0) {
            slot = &ctx->Slots[i];
        }
    }
    if (slot == NULL) {
        return VD_ERR_TOO_MANY_DEVICES;
    }
    if (*mdl != NULL) {
        ringMemory = MmGetSystemAddressForMdlSafe(*mdl, NormalPagePriority | MdlMappingNoExecute);
        ringLength = MmGetMdlByteCount(*mdl);
    }

    result = vd_core_command(&ctx->Core, owner, VD_CMD_CREATE_DEVICE, input, inputLength,
                             ringMemory, ringLength, reply, replyCap, replyLength);
    if (result != VD_OK) {
        return result;
    }

    RtlCopyMemory(&request, input, sizeof request); /* the core validated it */
    RtlCopyMemory(&created, reply, sizeof created);
    if (!NT_SUCCESS(VdCircuitDeviceAdd(ctx, created.device_id, &request, &circuitDevice))) {
        /* Undo through the core's own Remove Device, then report the failure. */
        vd_remove_device_request undo = { created.device_id };
        size_t ignored;
        (void)vd_core_command(&ctx->Core, owner, VD_CMD_REMOVE_DEVICE, &undo, sizeof undo,
                              NULL, 0, NULL, 0, &ignored);
        *replyLength = 0;
        return VD_ERR_SYSTEM;
    }

    /* The slot's reference keeps the circuit device's context (and its rundown)
     * valid until teardown, however Windows deletes the device meanwhile. */
    WdfObjectReference(circuitDevice);
    slot->DeviceId = created.device_id;
    slot->Mdl = *mdl;
    slot->CircuitDevice = circuitDevice;
    *mdl = NULL; /* the slot owns the pinned Ring now */
    return VD_OK;
}

static VOID VdEvtIoDeviceControl(WDFQUEUE Queue, WDFREQUEST Request, size_t OutputBufferLength,
                                 size_t InputBufferLength, ULONG IoControlCode)
{
    VD_DRIVER_CONTEXT *ctx = VdDriverContext(WdfGetDriver());
    vd_owner *owner = &VdFileContext(WdfRequestGetFileObject(Request))->Owner;
    VD_REQUEST_CONTEXT *requestCtx = VdRequestContext(Request);
    UCHAR input[sizeof(vd_create_device_request)]; /* the largest request */
    vd_reply_envelope envelope;
    UCHAR *output;
    PVOID buffer;
    size_t length;
    size_t replyLength = 0;
    uint32_t command;

    UNREFERENCED_PARAMETER(Queue);
    UNREFERENCED_PARAMETER(OutputBufferLength);

    if (!VdCommandFromIoctl(IoControlCode, &command)) {
        WdfRequestComplete(Request, STATUS_INVALID_DEVICE_REQUEST);
        return;
    }
    if (!NT_SUCCESS(WdfRequestRetrieveOutputBuffer(Request, sizeof envelope, &buffer, &length)) ||
        length < sizeof envelope) {
        WdfRequestComplete(Request, STATUS_BUFFER_TOO_SMALL);
        return;
    }
    output = (UCHAR *)buffer;

    /* Buffered I/O shares one buffer for input and output: copy the input first. */
    RtlZeroMemory(input, sizeof input);
    if (InputBufferLength > 0 &&
        NT_SUCCESS(WdfRequestRetrieveInputBuffer(Request, 0, &buffer, &InputBufferLength))) {
        RtlCopyMemory(input, buffer, min(InputBufferLength, sizeof input));
    }

    WdfWaitLockAcquire(ctx->Lock, NULL);
    if (command == VD_CMD_CREATE_DEVICE) {
        envelope.status = VdCreateDevice(ctx, owner, input, InputBufferLength, &requestCtx->Mdl,
                                         output + sizeof envelope, length - sizeof envelope,
                                         &replyLength);
    } else {
        envelope.status = vd_core_command(&ctx->Core, owner, command, input, InputBufferLength,
                                          NULL, 0, output + sizeof envelope,
                                          length - sizeof envelope, &replyLength);
        if (command == VD_CMD_REMOVE_DEVICE && envelope.status == VD_OK) {
            vd_remove_device_request removed;
            RtlCopyMemory(&removed, input, sizeof removed);
            VdDeviceTeardown(ctx, removed.device_id);
        }
    }
    WdfWaitLockRelease(ctx->Lock);

    RtlCopyMemory(output, &envelope, sizeof envelope);
    WdfRequestCompleteWithInformation(Request, STATUS_SUCCESS, sizeof envelope + replyLength);
}
