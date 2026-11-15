/*
 * The control device: where Owners send commands (ADR 0010, ADR 0011). Transport
 * only: a Device's lifecycle is endpoint.c's.
 *
 * One open handle is one Owner. Create Device's Ring is pinned while still in
 * the caller's own context, because its address only means something there
 * (ADR 0006, ADR 0009). Every command then runs under the driver lock, through
 * endpoint.c for Create and Remove Device and straight to the core otherwise, and
 * the reply goes back in the envelope from the ABI header.
 */
#include "shell.h"

static EVT_WDF_DEVICE_FILE_CREATE PattieEvtFileCreate;
static EVT_WDF_FILE_CLEANUP PattieEvtFileCleanup;
static EVT_WDF_IO_IN_CALLER_CONTEXT PattieEvtIoInCallerContext;
static EVT_WDF_IO_QUEUE_IO_DEVICE_CONTROL PattieEvtIoDeviceControl;
static EVT_WDF_OBJECT_CONTEXT_CLEANUP PattieEvtRequestCleanup;

/* SYSTEM and Administrators: full access. Interactive users: read/write. Nothing else. */
DECLARE_CONST_UNICODE_STRING(PattieControlSddl, L"D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)");
DECLARE_CONST_UNICODE_STRING(PattieControlName, L"\\Device\\Pattie");
DECLARE_CONST_UNICODE_STRING(PattieControlLink, L"\\DosDevices\\Pattie");

_Use_decl_annotations_
NTSTATUS PattieControlDeviceCreate(WDFDRIVER Driver)
{
    PATTIE_DRIVER_CONTEXT *ctx = PattieDriverContext(Driver);
    PWDFDEVICE_INIT init;
    WDF_FILEOBJECT_CONFIG fileConfig;
    WDF_OBJECT_ATTRIBUTES fileAttributes;
    WDF_OBJECT_ATTRIBUTES requestAttributes;
    WDF_OBJECT_ATTRIBUTES deviceAttributes;
    WDF_IO_QUEUE_CONFIG queueConfig;
    WDFDEVICE control;
    NTSTATUS status;

    init = WdfControlDeviceInitAllocate(Driver, &PattieControlSddl);
    if (init == NULL) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    WdfDeviceInitSetExclusive(init, FALSE);
    WdfDeviceInitSetIoType(init, WdfDeviceIoBuffered);
    status = WdfDeviceInitAssignName(init, &PattieControlName);
    if (!NT_SUCCESS(status)) {
        WdfDeviceInitFree(init);
        return status;
    }

    WDF_FILEOBJECT_CONFIG_INIT(&fileConfig, PattieEvtFileCreate, WDF_NO_EVENT_CALLBACK, PattieEvtFileCleanup);
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&fileAttributes, PATTIE_FILE_CONTEXT);
    WdfDeviceInitSetFileObjectConfig(init, &fileConfig, &fileAttributes);

    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&requestAttributes, PATTIE_REQUEST_CONTEXT);
    requestAttributes.EvtCleanupCallback = PattieEvtRequestCleanup;
    WdfDeviceInitSetRequestAttributes(init, &requestAttributes);
    WdfDeviceInitSetIoInCallerContextCallback(init, PattieEvtIoInCallerContext);

    WDF_OBJECT_ATTRIBUTES_INIT(&deviceAttributes);
    deviceAttributes.ExecutionLevel = WdfExecutionLevelPassive; /* waitlock, MDL work */
    status = WdfDeviceCreate(&init, &deviceAttributes, &control);
    if (!NT_SUCCESS(status)) {
        WdfDeviceInitFree(init);
        return status;
    }

    status = WdfDeviceCreateSymbolicLink(control, &PattieControlLink);
    if (!NT_SUCCESS(status)) {
        WdfObjectDelete(control);
        return status;
    }

    /* Sequential: one command at a time reaches the core (ADR 0002). */
    WDF_IO_QUEUE_CONFIG_INIT_DEFAULT_QUEUE(&queueConfig, WdfIoQueueDispatchSequential);
    queueConfig.EvtIoDeviceControl = PattieEvtIoDeviceControl;
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
VOID PattieUnpinRing(PMDL Mdl)
{
    MmUnlockPages(Mdl);
    IoFreeMdl(Mdl);
}

/*
 * One open is one Owner. Only the device itself may be opened: `\\.\Pattie`,
 * never `\\.\Pattie\anything`. KMDF sets FILE_DEVICE_SECURE_OPEN, so the ACL
 * applies to such opens anyway; refusing them keeps that from resting on a
 * framework default (security review 2026-11-14, finding 5).
 */
static VOID PattieEvtFileCreate(WDFDEVICE Device, WDFREQUEST Request, WDFFILEOBJECT FileObject)
{
    PUNICODE_STRING name = WdfFileObjectGetFileName(FileObject);

    UNREFERENCED_PARAMETER(Device);
    if (name != NULL && name->Length != 0) {
        WdfRequestComplete(Request, STATUS_OBJECT_NAME_NOT_FOUND);
        return;
    }
    pattie_owner_init(&PattieFileContext(FileObject)->Owner);
    InterlockedIncrement(&PattieDriverContext(WdfGetDriver())->OpenOwners); /* until cleanup */
    WdfRequestComplete(Request, STATUS_SUCCESS);
}

/* The Owner's last handle closed, crash included: remove all its Devices (ADR 0005). */
static VOID PattieEvtFileCleanup(WDFFILEOBJECT FileObject)
{
    PATTIE_DRIVER_CONTEXT *ctx = PattieDriverContext(WdfGetDriver());

    WdfWaitLockAcquire(ctx->Lock, NULL);
    PattieOwnerDevicesRemove(ctx, &PattieFileContext(FileObject)->Owner);
    WdfWaitLockRelease(ctx->Lock);
    InterlockedDecrement(&ctx->OpenOwners); /* counted in PattieEvtFileCreate */
}

/* Pins the Ring of a Create Device request; anything that can't be pinned reaches
 * the core as "no Ring", which the core rejects (ADR 0009). */
static PMDL PattieTryPinRing(WDFREQUEST Request)
{
    pattie_create_device_request *request;
    size_t length;
    PMDL mdl;

    if (!NT_SUCCESS(WdfRequestRetrieveInputBuffer(Request, sizeof *request, (PVOID *)&request, &length))) {
        return NULL;
    }
    if (request->ring_address == 0 || request->ring_bytes == 0 ||
        request->ring_bytes > PATTIE_RING_MAX_BYTES) {
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

/*
 * References the doorbell event a Create Device request names (ADR 0021), as
 * Microsoft's event sample does: only an event, only with the right to set it,
 * checked against the caller's own handle table. NULL if it isn't one; Create
 * Device then fails with "bad request".
 */
static PKEVENT PattieTryReferenceWake(WDFREQUEST Request)
{
    pattie_create_device_request *request;
    size_t length;
    PKEVENT event;

    if (!NT_SUCCESS(WdfRequestRetrieveInputBuffer(Request, sizeof *request, (PVOID *)&request, &length))) {
        return NULL;
    }
    if (request->wake_event == 0 ||
        !NT_SUCCESS(ObReferenceObjectByHandle((HANDLE)(ULONG_PTR)request->wake_event,
                                              EVENT_MODIFY_STATE, *ExEventObjectType,
                                              WdfRequestGetRequestorMode(Request),
                                              (PVOID *)&event, NULL))) {
        return NULL;
    }
    return event;
}

static VOID PattieEvtIoInCallerContext(WDFDEVICE Device, WDFREQUEST Request)
{
    PATTIE_DRIVER_CONTEXT *ctx = PattieDriverContext(WdfGetDriver());
    WDF_REQUEST_PARAMETERS params;
    NTSTATUS status;

    WDF_REQUEST_PARAMETERS_INIT(&params);
    WdfRequestGetParameters(Request, &params);
    if (params.Type == WdfRequestTypeDeviceControl &&
        params.Parameters.DeviceIoControl.IoControlCode == PATTIE_IOCTL(PATTIE_CMD_CREATE_DEVICE)) {
        /* Bounded: no app may hold more than a few pinned Rings in flight. */
        if ((ULONG)InterlockedIncrement(&ctx->PinnedRequests) > PATTIE_MAX_PINNED_REQUESTS) {
            InterlockedDecrement(&ctx->PinnedRequests);
            WdfRequestComplete(Request, STATUS_DEVICE_BUSY);
            return;
        }
        PattieRequestContext(Request)->CountedPinned = TRUE; /* released in request cleanup */
        PattieRequestContext(Request)->Mdl = PattieTryPinRing(Request);
        PattieRequestContext(Request)->WakeEvent = PattieTryReferenceWake(Request);
    }

    status = WdfDeviceEnqueueRequest(Device, Request);
    if (!NT_SUCCESS(status)) {
        WdfRequestComplete(Request, status); /* request cleanup unpins */
    }
}

static VOID PattieEvtRequestCleanup(WDFOBJECT Object)
{
    PATTIE_REQUEST_CONTEXT *requestCtx = PattieRequestContext((WDFREQUEST)Object);

    if (requestCtx->Mdl != NULL) {
        PattieUnpinRing(requestCtx->Mdl);
        requestCtx->Mdl = NULL;
    }
    if (requestCtx->WakeEvent != NULL) {
        ObDereferenceObject(requestCtx->WakeEvent);
        requestCtx->WakeEvent = NULL;
    }
    if (requestCtx->CountedPinned) {
        InterlockedDecrement(&PattieDriverContext(WdfGetDriver())->PinnedRequests);
        requestCtx->CountedPinned = FALSE;
    }
}

static BOOLEAN PattieCommandFromIoctl(ULONG IoControlCode, uint32_t *Command)
{
    static const uint32_t commands[] = {
        PATTIE_CMD_HELLO, PATTIE_CMD_CREATE_DEVICE, PATTIE_CMD_REMOVE_DEVICE,
    };

    for (ULONG i = 0; i < ARRAYSIZE(commands); i++) {
        if (IoControlCode == PATTIE_IOCTL(commands[i])) {
            *Command = commands[i];
            return TRUE;
        }
    }
    return FALSE;
}

static VOID PattieEvtIoDeviceControl(WDFQUEUE Queue, WDFREQUEST Request, size_t OutputBufferLength,
                                 size_t InputBufferLength, ULONG IoControlCode)
{
    PATTIE_DRIVER_CONTEXT *ctx = PattieDriverContext(WdfGetDriver());
    pattie_owner *owner = &PattieFileContext(WdfRequestGetFileObject(Request))->Owner;
    PATTIE_REQUEST_CONTEXT *requestCtx = PattieRequestContext(Request);
    UCHAR input[sizeof(pattie_create_device_request)]; /* the largest request */
    pattie_reply_envelope envelope;
    UCHAR *output;
    PVOID buffer;
    size_t length;
    size_t replyLength = 0;
    uint32_t command;

    UNREFERENCED_PARAMETER(Queue);
    UNREFERENCED_PARAMETER(OutputBufferLength);

    if (!PattieCommandFromIoctl(IoControlCode, &command)) {
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
    if (ctx->Root == NULL) {
        /*
         * The root was removed with this Owner's handle still open (a forced
         * removal the veto couldn't stop; ADR 0024). Its Devices are gone and
         * none can be made: fail the command itself, which the SDK reports as
         * "driver removed" (ERROR_DEVICE_REMOVED).
         */
        WdfWaitLockRelease(ctx->Lock);
        WdfRequestComplete(Request, STATUS_DEVICE_REMOVED);
        return;
    }
    if (command == PATTIE_CMD_CREATE_DEVICE) {
        envelope.status = PattieDeviceCreate(ctx, owner, input, InputBufferLength, &requestCtx->Mdl,
                                         &requestCtx->WakeEvent, output + sizeof envelope,
                                         length - sizeof envelope, &replyLength);
    } else if (command == PATTIE_CMD_REMOVE_DEVICE) {
        envelope.status = PattieDeviceRemove(ctx, owner, input, InputBufferLength, output + sizeof envelope,
                                         length - sizeof envelope, &replyLength);
    } else {
        envelope.status = pattie_core_command(&ctx->Core, owner, command, input, InputBufferLength,
                                          NULL, 0, output + sizeof envelope,
                                          length - sizeof envelope, &replyLength);
    }
    WdfWaitLockRelease(ctx->Lock);

    RtlCopyMemory(output, &envelope, sizeof envelope);
    WdfRequestCompleteWithInformation(Request, STATUS_SUCCESS, sizeof envelope + replyLength);
}
