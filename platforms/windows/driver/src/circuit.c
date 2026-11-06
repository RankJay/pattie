/*
 * The circuit inside a circuit device: one capture (Microphone) or render
 * (Speaker) circuit with a host pin, where Windows' audio engine streams, and a
 * bridge pin, the "physical" microphone or speaker end (ADR 0010).
 *
 * Built like the circuits in Microsoft's ACX samples (AudioCodec's capture and
 * render circuits): same pin types, communication and categories.
 */
#include "shell.h"

/*
 * The circuit's name is also the reference string of its audio interfaces
 * (interfaces.c), so both take it from here.
 */
DECLARE_CONST_UNICODE_STRING(VdMicrophoneCircuitName, L"Microphone0");
DECLARE_CONST_UNICODE_STRING(VdSpeakerCircuitName, L"Speaker0");

/*
 * The host pin's formats (ADR 0005), both 48 kHz mono, offered in this order.
 * 32-bit float is the Ring's own format and is copied as is. 16-bit PCM is what
 * Windows' audio endpoint builder needs as the device format: offering float
 * alone left the endpoint NOTPRESENT (VM, 2026-11-07). stream.c converts it with
 * the core's integer-only conversion.
 */
static KSDATAFORMAT_WAVEFORMATEXTENSIBLE VdFormatFloat = {
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
            1,                                              /* channels */
            VD_SAMPLE_RATE,
            VD_SAMPLE_RATE * VD_FLOAT_BYTES_PER_FRAME,      /* bytes per second */
            VD_FLOAT_BYTES_PER_FRAME,                       /* block align */
            32,                                             /* bits per sample */
            sizeof(WAVEFORMATEXTENSIBLE) - sizeof(WAVEFORMATEX),
        },
        { 32 },                                             /* valid bits per sample */
        KSAUDIO_SPEAKER_MONO,
        STATICGUIDOF(KSDATAFORMAT_SUBTYPE_IEEE_FLOAT),
    },
};

static KSDATAFORMAT_WAVEFORMATEXTENSIBLE VdFormatPcm16 = {
    {
        sizeof(KSDATAFORMAT_WAVEFORMATEXTENSIBLE),
        0,
        0,
        0,
        STATICGUIDOF(KSDATAFORMAT_TYPE_AUDIO),
        STATICGUIDOF(KSDATAFORMAT_SUBTYPE_PCM),
        STATICGUIDOF(KSDATAFORMAT_SPECIFIER_WAVEFORMATEX),
    },
    {
        {
            WAVE_FORMAT_EXTENSIBLE,
            1,                                              /* channels */
            VD_SAMPLE_RATE,
            VD_SAMPLE_RATE * VD_PCM16_BYTES_PER_FRAME,      /* bytes per second */
            VD_PCM16_BYTES_PER_FRAME,                       /* block align */
            16,                                             /* bits per sample */
            sizeof(WAVEFORMATEXTENSIBLE) - sizeof(WAVEFORMATEX),
        },
        { 16 },                                             /* valid bits per sample */
        KSAUDIO_SPEAKER_MONO,
        STATICGUIDOF(KSDATAFORMAT_SUBTYPE_PCM),
    },
};

_Use_decl_annotations_
PCUNICODE_STRING VdCircuitName(UINT32 Kind)
{
    return Kind == VD_KIND_MICROPHONE ? &VdMicrophoneCircuitName : &VdSpeakerCircuitName;
}

/* Adds one format, owned by the circuit, to the host pin's list. */
static NTSTATUS VdAddFormat(WDFDEVICE Device, ACXCIRCUIT Circuit, ACXDATAFORMATLIST List,
                            KSDATAFORMAT_WAVEFORMATEXTENSIBLE *Format)
{
    ACX_DATAFORMAT_CONFIG formatConfig;
    WDF_OBJECT_ATTRIBUTES attributes;
    ACXDATAFORMAT format;
    NTSTATUS status;

    ACX_DATAFORMAT_CONFIG_INIT_KS(&formatConfig, Format);
    WDF_OBJECT_ATTRIBUTES_INIT(&attributes);
    attributes.ParentObject = Circuit;
    status = AcxDataFormatCreate(Device, &attributes, &formatConfig, &format);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    return AcxDataFormatListAddDataFormat(List, format);
}

/* Creates one pin owned by the circuit. */
static NTSTATUS VdCreatePin(ACXCIRCUIT Circuit, ACX_PIN_TYPE Type, ACX_PIN_COMMUNICATION Communication,
                            const GUID *Category, ACXPIN *Pin)
{
    ACX_PIN_CONFIG pinConfig;
    WDF_OBJECT_ATTRIBUTES attributes;

    ACX_PIN_CONFIG_INIT(&pinConfig);
    pinConfig.Type = Type;
    pinConfig.Communication = Communication;
    pinConfig.Category = Category;
    WDF_OBJECT_ATTRIBUTES_INIT(&attributes);
    attributes.ParentObject = Circuit;
    return AcxPinCreate(Circuit, &attributes, &pinConfig, Pin);
}

/*
 * Creates the device's one circuit and stores it in DeviceCtx->Circuit. Called
 * with the device, before ACX sees it (endpoint.c); added to the device when it
 * first prepares its hardware.
 */
_Use_decl_annotations_
NTSTATUS VdCircuitCreate(WDFDEVICE Device, VD_CIRCUIT_DEVICE_CONTEXT *DeviceCtx)
{
    BOOLEAN microphone = DeviceCtx->Kind == VD_KIND_MICROPHONE;
    PACXCIRCUIT_INIT circuitInit;
    WDF_OBJECT_ATTRIBUTES attributes;
    ACXCIRCUIT circuit;
    ACXPIN pins[2];
    ACXDATAFORMATLIST formatList;
    NTSTATUS status;

    circuitInit = AcxCircuitInitAllocate(Device);
    if (circuitInit == NULL) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    AcxCircuitInitSetComponentId(circuitInit, &VIADUCT_COMPONENT_GUID);
    status = AcxCircuitInitAssignName(circuitInit, VdCircuitName(DeviceCtx->Kind));
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
    status = VdCreatePin(circuit, microphone ? AcxPinTypeSource : AcxPinTypeSink,
                         AcxPinCommunicationSink, &KSCATEGORY_AUDIO, &pins[0]);
    if (NT_SUCCESS(status)) {
        /* Bridge pin: the "physical" end, a microphone or speaker. */
        status = VdCreatePin(circuit, microphone ? AcxPinTypeSink : AcxPinTypeSource,
                             AcxPinCommunicationNone,
                             microphone ? &KSNODETYPE_MICROPHONE : &KSNODETYPE_SPEAKER, &pins[1]);
    }
    if (!NT_SUCCESS(status)) {
        return status;
    }

    formatList = AcxPinGetRawDataFormatList(pins[0]);
    if (formatList == NULL) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    /* Float first, then PCM16: the order the VM endpoint went ACTIVE with. */
    status = VdAddFormat(Device, circuit, formatList, &VdFormatFloat);
    if (NT_SUCCESS(status)) {
        status = VdAddFormat(Device, circuit, formatList, &VdFormatPcm16);
    }
    if (NT_SUCCESS(status)) {
        status = AcxCircuitAddPins(circuit, pins, ARRAYSIZE(pins));
    }
    if (!NT_SUCCESS(status)) {
        return status;
    }
    DeviceCtx->Circuit = circuit;
    return STATUS_SUCCESS;
}
