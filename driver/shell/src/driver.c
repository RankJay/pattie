/*
 * Driver entry and the root ACX device (ADR 0010).
 *
 * The root device has no circuits of its own: it parents one circuit device per
 * Device. The control device that Owners talk to is created with the root and
 * deleted with it, so a PnP removal of the root lets the driver unload.
 */
#include "shell.h"

static EVT_WDF_DRIVER_DEVICE_ADD VdEvtRootDeviceAdd;
static EVT_WDF_OBJECT_CONTEXT_CLEANUP VdEvtRootCleanup;

NTSTATUS DriverEntry(PDRIVER_OBJECT DriverObject, PUNICODE_STRING RegistryPath)
{
    WDF_DRIVER_CONFIG config;
    WDF_OBJECT_ATTRIBUTES attributes;
    ACX_DRIVER_CONFIG acxConfig;
    WDFDRIVER driver;
    VD_DRIVER_CONTEXT *ctx;
    NTSTATUS status;

    WDF_DRIVER_CONFIG_INIT(&config, VdEvtRootDeviceAdd);
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, VD_DRIVER_CONTEXT);
    status = WdfDriverCreate(DriverObject, RegistryPath, &attributes, &config, &driver);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    ctx = VdDriverContext(driver); /* zeroed by WDF */
    vd_core_init(&ctx->Core);
    status = WdfWaitLockCreate(WDF_NO_OBJECT_ATTRIBUTES, &ctx->Lock);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    ACX_DRIVER_CONFIG_INIT(&acxConfig);
    return AcxDriverInitialize(driver, &acxConfig);
}

static NTSTATUS VdEvtRootDeviceAdd(WDFDRIVER Driver, PWDFDEVICE_INIT DeviceInit)
{
    VD_DRIVER_CONTEXT *ctx = VdDriverContext(Driver);
    ACX_DEVICEINIT_CONFIG acxInitConfig;
    ACX_DEVICE_CONFIG acxConfig;
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

    WDF_OBJECT_ATTRIBUTES_INIT(&attributes);
    attributes.EvtCleanupCallback = VdEvtRootCleanup;
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
    status = VdControlDeviceCreate(Driver);
    if (!NT_SUCCESS(status)) {
        ctx->Root = NULL;
    }
    return status;
}

static VOID VdEvtRootCleanup(WDFOBJECT Object)
{
    VD_DRIVER_CONTEXT *ctx = VdDriverContext(WdfGetDriver());

    UNREFERENCED_PARAMETER(Object);

    WdfWaitLockAcquire(ctx->Lock, NULL);
    ctx->Root = NULL;
    if (ctx->Control != NULL) {
        WdfObjectDelete(ctx->Control);
        ctx->Control = NULL;
    }
    WdfWaitLockRelease(ctx->Lock);
}
