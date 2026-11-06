/*
 * Circuit devices: one raw child of the root per live Device (ADR 0010).
 *
 * Runtime endpoints follow ACX's dynamic-circuit pattern: allocate a child with
 * WdfPdoInitAllocate, create it, hand it to ACX with AcxDeviceAddCircuitDevice,
 * and build its one circuit when it prepares hardware. Removal asks ACX to remove
 * the child, which completes asynchronously.
 */
#include "shell.h"

static EVT_WDF_DEVICE_PREPARE_HARDWARE VdEvtCircuitDevicePrepareHardware;
static EVT_WDF_DEVICE_RELEASE_HARDWARE VdEvtCircuitDeviceReleaseHardware;
static EVT_WDF_OBJECT_CONTEXT_CLEANUP VdEvtCircuitDeviceCleanup;

DECLARE_CONST_UNICODE_STRING(VdCircuitDeviceId, L"Viaduct\\Endpoint");
DECLARE_CONST_UNICODE_STRING(VdMicrophoneCircuitName, L"Microphone0");
DECLARE_CONST_UNICODE_STRING(VdSpeakerCircuitName, L"Speaker0");

/* The one format v0 supports: 48 kHz, mono, 32-bit float (ADR 0005). */
static KSDATAFORMAT_WAVEFORMATEXTENSIBLE VdFormat = {
    {
        sizeof(KSDATAFORMAT_WAVEFORMATEXTENSIBLE),
        0,
        0,
        0,
        STATICGUIDOF(KSDATAFORMAT_TYPE_AUDIO),
        STATICGUIDOF(KSDATAFORMAT_SUBTYPE_IEEE_FLOAT),
        STATICGUIDOF(KSDATAFORMAT_SPECIFIER_WAVEFORMATEX),
    },
    {
        {
            WAVE_FORMAT_EXTENSIBLE,
            1,                                       /* channels */
            VD_SAMPLE_RATE,
            VD_SAMPLE_RATE * VD_BYTES_PER_FRAME,     /* bytes per second */
            VD_BYTES_PER_FRAME,                      /* block align */
            32,                                      /* bits per sample */
            sizeof(WAVEFORMATEXTENSIBLE) - sizeof(WAVEFORMATEX),
        },
        { 32 },                                      /* valid bits per sample */
        KSAUDIO_SPEAKER_MONO,
        STATICGUIDOF(KSDATAFORMAT_SUBTYPE_IEEE_FLOAT),
    },
};

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
    DECLARE_UNICODE_STRING_SIZE(instanceId, 16);
    UNICODE_STRING name;
    WDFDEVICE device;
    NTSTATUS status;

    *CircuitDevice = NULL;

    init = WdfPdoInitAllocate(Ctx->Root);
    if (init == NULL) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }

    name.Buffer = (PWCH)Request->name;
    name.Length = (USHORT)(Request->name_len * sizeof(WCHAR));
    name.MaximumLength = name.Length;

    status = RtlUnicodeStringPrintf(&instanceId, L"%u", DeviceId);
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
        return status;
    }

    WDF_PNPPOWER_EVENT_CALLBACKS_INIT(&pnpCallbacks);
    pnpCallbacks.EvtDevicePrepareHardware = VdEvtCircuitDevicePrepareHardware;
    pnpCallbacks.EvtDeviceReleaseHardware = VdEvtCircuitDeviceReleaseHardware;
    WdfDeviceInitSetPnpPowerEventCallbacks(init, &pnpCallbacks);

    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, VD_CIRCUIT_DEVICE_CONTEXT);
    attributes.EvtCleanupCallback = VdEvtCircuitDeviceCleanup;
    status = WdfDeviceCreate(&init, &attributes, &device);
    if (!NT_SUCCESS(status)) {
        WdfDeviceInitFree(init);
        return status;
    }

    deviceCtx = VdCircuitDeviceContext(device);
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
        status = AcxDeviceAddCircuitDevice(Ctx->Root, device);
    }
    if (!NT_SUCCESS(status)) {
        WdfObjectDelete(device);
        return status;
    }

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
        return;
    }
}

/*
 * Windows is deleting the circuit device. Takes no lock (teardown may hold the
 * driver lock while asking ACX to remove this device): it only records the fact.
 * The slot's reference keeps this context valid until teardown releases it.
 */
static VOID VdEvtCircuitDeviceCleanup(WDFOBJECT Object)
{
    InterlockedExchange(&VdCircuitDeviceContext((WDFDEVICE)Object)->Deleted, 1);
}

static NTSTATUS VdCreateCircuit(WDFDEVICE Device, VD_CIRCUIT_DEVICE_CONTEXT *DeviceCtx)
{
    BOOLEAN microphone = DeviceCtx->Kind == VD_KIND_MICROPHONE;
    PACXCIRCUIT_INIT circuitInit;
    ACX_PIN_CONFIG pinConfig;
    ACX_DATAFORMAT_CONFIG formatConfig;
    WDF_OBJECT_ATTRIBUTES attributes;
    ACXCIRCUIT circuit;
    ACXPIN pins[2];
    ACXDATAFORMAT format;
    ACXDATAFORMATLIST formatList;
    NTSTATUS status;

    circuitInit = AcxCircuitInitAllocate(Device);
    if (circuitInit == NULL) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    AcxCircuitInitSetComponentId(circuitInit, &VIADUCT_COMPONENT_GUID);
    status = AcxCircuitInitAssignName(circuitInit,
                                      microphone ? &VdMicrophoneCircuitName : &VdSpeakerCircuitName);
    if (NT_SUCCESS(status)) {
        AcxCircuitInitSetCircuitType(circuitInit, microphone ? AcxCircuitTypeCapture : AcxCircuitTypeRender);
        status = AcxCircuitInitAssignAcxCreateStreamCallback(circuitInit, VdEvtCircuitCreateStream);
    }
    if (!NT_SUCCESS(status)) {
        AcxCircuitInitFree(circuitInit);
        return status;
    }

    WDF_OBJECT_ATTRIBUTES_INIT(&attributes);
    status = AcxCircuitCreate(Device, &attributes, &circuitInit, &circuit);
    if (!NT_SUCCESS(status)) {
        return status; /* ACX frees circuitInit on failure */
    }

    /* Host pin: where Windows' audio engine streams. */
    ACX_PIN_CONFIG_INIT(&pinConfig);
    pinConfig.Type = microphone ? AcxPinTypeSource : AcxPinTypeSink;
    pinConfig.Communication = AcxPinCommunicationSink;
    pinConfig.Category = &KSCATEGORY_AUDIO;
    WDF_OBJECT_ATTRIBUTES_INIT(&attributes);
    attributes.ParentObject = circuit;
    status = AcxPinCreate(circuit, &attributes, &pinConfig, &pins[0]);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    /* Bridge pin: the "physical" end, a microphone or speaker. */
    ACX_PIN_CONFIG_INIT(&pinConfig);
    pinConfig.Type = microphone ? AcxPinTypeSink : AcxPinTypeSource;
    pinConfig.Communication = AcxPinCommunicationNone;
    pinConfig.Category = microphone ? &KSNODETYPE_MICROPHONE : &KSNODETYPE_SPEAKER;
    WDF_OBJECT_ATTRIBUTES_INIT(&attributes);
    attributes.ParentObject = circuit;
    status = AcxPinCreate(circuit, &attributes, &pinConfig, &pins[1]);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    ACX_DATAFORMAT_CONFIG_INIT_KS(&formatConfig, &VdFormat);
    WDF_OBJECT_ATTRIBUTES_INIT(&attributes);
    attributes.ParentObject = circuit;
    status = AcxDataFormatCreate(Device, &attributes, &formatConfig, &format);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    formatList = AcxPinGetRawDataFormatList(pins[0]);
    if (formatList == NULL) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    status = AcxDataFormatListAddDataFormat(formatList, format);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    status = AcxCircuitAddPins(circuit, pins, ARRAYSIZE(pins));
    if (!NT_SUCCESS(status)) {
        return status;
    }
    DeviceCtx->Circuit = circuit;
    return STATUS_SUCCESS;
}

static NTSTATUS VdEvtCircuitDevicePrepareHardware(WDFDEVICE Device, WDFCMRESLIST Resources,
                                                  WDFCMRESLIST ResourcesTranslated)
{
    VD_CIRCUIT_DEVICE_CONTEXT *deviceCtx = VdCircuitDeviceContext(Device);
    NTSTATUS status;

    UNREFERENCED_PARAMETER(Resources);
    UNREFERENCED_PARAMETER(ResourcesTranslated);

    if (deviceCtx->Circuit == NULL) {
        status = VdCreateCircuit(Device, deviceCtx);
        if (!NT_SUCCESS(status)) {
            return status;
        }
    }
    if (deviceCtx->Circuit == NULL) {
        return STATUS_INVALID_DEVICE_STATE;
    }
    return AcxDeviceAddCircuit(Device, deviceCtx->Circuit);
}

static NTSTATUS VdEvtCircuitDeviceReleaseHardware(WDFDEVICE Device, WDFCMRESLIST ResourcesTranslated)
{
    VD_CIRCUIT_DEVICE_CONTEXT *deviceCtx = VdCircuitDeviceContext(Device);

    UNREFERENCED_PARAMETER(ResourcesTranslated);

    if (deviceCtx->Circuit != NULL) {
        return AcxDeviceRemoveCircuit(Device, deviceCtx->Circuit);
    }
    return STATUS_SUCCESS;
}
