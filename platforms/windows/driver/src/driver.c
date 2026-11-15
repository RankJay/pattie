/*
 * Driver entry and the root ACX device (ADR 0010).
 *
 * The root device has no circuits of its own: it parents one circuit device per
 * Device. The control device that Owners talk to is created with the root and
 * deleted with it, so a PnP removal of the root lets the driver unload; the root
 * refuses that removal while an Owner has the control device open (ADR 0024).
 */
#include "shell.h"

static EVT_WDF_DRIVER_DEVICE_ADD PattieEvtRootDeviceAdd;
static EVT_WDF_DRIVER_UNLOAD PattieEvtDriverUnload;
static EVT_WDF_OBJECT_CONTEXT_CLEANUP PattieEvtRootCleanup;
static EVT_WDF_DEVICE_QUERY_REMOVE PattieEvtRootQueryRemove;

/*
 * The trace provider is registered before anything else and unregistered if
 * anything after it fails: WDF never calls the unload callback when DriverEntry
 * fails (trace.c).
 */
NTSTATUS DriverEntry(PDRIVER_OBJECT DriverObject, PUNICODE_STRING RegistryPath)
{
    WDF_DRIVER_CONFIG config;
    WDF_OBJECT_ATTRIBUTES attributes;
    ACX_DRIVER_CONFIG acxConfig;
    WDFDRIVER driver;
    PATTIE_DRIVER_CONTEXT *ctx;
    NTSTATUS status;

    status = TraceLoggingRegister(PattieTraceProvider);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    WDF_DRIVER_CONFIG_INIT(&config, PattieEvtRootDeviceAdd);
    config.EvtDriverUnload = PattieEvtDriverUnload;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, PATTIE_DRIVER_CONTEXT);
    status = WdfDriverCreate(DriverObject, RegistryPath, &attributes, &config, &driver);
    if (NT_SUCCESS(status)) {
        ctx = PattieDriverContext(driver); /* zeroed by WDF */
        pattie_core_init(&ctx->Core);
        status = WdfWaitLockCreate(WDF_NO_OBJECT_ATTRIBUTES, &ctx->Lock);
    }
    if (NT_SUCCESS(status)) {
        ACX_DRIVER_CONFIG_INIT(&acxConfig);
        status = AcxDriverInitialize(driver, &acxConfig);
    }
    if (!NT_SUCCESS(status)) {
        TraceLoggingUnregister(PattieTraceProvider);
    }
    return status;
}

static VOID PattieEvtDriverUnload(WDFDRIVER Driver)
{
    UNREFERENCED_PARAMETER(Driver);

    TraceLoggingUnregister(PattieTraceProvider);
}

static NTSTATUS PattieEvtRootDeviceAdd(WDFDRIVER Driver, PWDFDEVICE_INIT DeviceInit)
{
    PATTIE_DRIVER_CONTEXT *ctx = PattieDriverContext(Driver);
    ACX_DEVICEINIT_CONFIG acxInitConfig;
    ACX_DEVICE_CONFIG acxConfig;
    WDF_PNPPOWER_EVENT_CALLBACKS pnpPowerCallbacks;
    WDF_OBJECT_ATTRIBUTES attributes;
    WDFDEVICE device;
    NTSTATUS status;

    if (ctx->Root != NULL) {
        return STATUS_DEVICE_ALREADY_ATTACHED; /* exactly one root (ADR 0010) */
    }

    ACX_DEVICEINIT_CONFIG_INIT(&acxInitConfig);
    status = AcxDeviceInitInitialize(DeviceInit, &acxInitConfig);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    /* Set on the DeviceInit before WdfDeviceCreate, as Microsoft's ACX samples do. */
    WDF_PNPPOWER_EVENT_CALLBACKS_INIT(&pnpPowerCallbacks);
    pnpPowerCallbacks.EvtDeviceQueryRemove = PattieEvtRootQueryRemove;
    WdfDeviceInitSetPnpPowerEventCallbacks(DeviceInit, &pnpPowerCallbacks);

    WDF_OBJECT_ATTRIBUTES_INIT(&attributes);
    attributes.EvtCleanupCallback = PattieEvtRootCleanup;
    status = WdfDeviceCreate(&DeviceInit, &attributes, &device);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    ACX_DEVICE_CONFIG_INIT(&acxConfig);
    status = AcxDeviceInitialize(device, &acxConfig);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    ctx->Root = device;
    status = PattieControlDeviceCreate(Driver);
    if (!NT_SUCCESS(status)) {
        ctx->Root = NULL;
    }
    return status;
}

/*
 * A driver update, an uninstall or a disable asks first: refused while any
 * Owner has the control device open (ADR 0024). The control device isn't a PnP
 * device, so nothing else would stop Windows removing the root under it, which
 * left the old image loaded and the root at CM_PROB_NEED_RESTART (VM-STEPS R1,
 * case B). Refused, Windows finishes the update at the next restart and says
 * so (the reboot flag, or 3010 from the installer).
 *
 * An Owner that opens after this allows the removal is left to the gone path
 * (PattieEvtIoDeviceControl): a race the veto narrows but can't close, since WDF
 * has no callback for a canceled removal.
 */
static NTSTATUS PattieEvtRootQueryRemove(WDFDEVICE Device)
{
    PATTIE_DRIVER_CONTEXT *ctx = PattieDriverContext(WdfGetDriver());
    LONG owners = InterlockedCompareExchange(&ctx->OpenOwners, 0, 0);

    UNREFERENCED_PARAMETER(Device);
    if (owners > 0) {
        TraceLoggingWrite(PattieTraceProvider, "QueryRemoveVetoed",
                          TraceLoggingLevel(PATTIE_TRACE_LEVEL_INFO),
                          TraceLoggingInt32(owners, "OpenOwners"));
        return STATUS_DEVICE_BUSY;
    }
    return STATUS_SUCCESS;
}

static VOID PattieEvtRootCleanup(WDFOBJECT Object)
{
    PATTIE_DRIVER_CONTEXT *ctx = PattieDriverContext(WdfGetDriver());

    UNREFERENCED_PARAMETER(Object);

    WdfWaitLockAcquire(ctx->Lock, NULL);
    ctx->Root = NULL;
    if (ctx->Control != NULL) {
        WdfObjectDelete(ctx->Control);
        ctx->Control = NULL;
    }
    WdfWaitLockRelease(ctx->Lock);
}
