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
 * The host pin's formats (ADR 0005, ADR 0023): the Device's own rate and
 * channels, in two encodings, offered in this order. 32-bit float is the Ring's
 * own format and is copied as is. 16-bit PCM is what Windows' audio endpoint
 * builder needs as the device format: offering float alone left the endpoint
 * NOTPRESENT (VM, 2026-11-07). stream.c converts it with the core's integer-only
 * conversion. Apps in any other format are converted by Windows' audio engine.
 *
 * The channel layouts are the standard masks, in WAVEFORMATEXTENSIBLE's order;
 * the Linux backend offers the same positions (ADR 0023). Indexed by channels.
 */
static const DWORD VdChannelMasks[VD_RING_MAX_CHANNELS + 1] = {
    0,
    KSAUDIO_SPEAKER_MONO,                                           /* mono */
    KSAUDIO_SPEAKER_STEREO,                                         /* stereo */
    SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT | SPEAKER_FRONT_CENTER,/* 3.0 */
    KSAUDIO_SPEAKER_QUAD,                                           /* quad */
    SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT | SPEAKER_FRONT_CENTER |
        SPEAKER_BACK_LEFT | SPEAKER_BACK_RIGHT,                     /* 5.0 */
    KSAUDIO_SPEAKER_5POINT1,                                        /* 5.1 */
    KSAUDIO_SPEAKER_5POINT1 | SPEAKER_BACK_CENTER,                  /* 6.1 */
    KSAUDIO_SPEAKER_7POINT1_SURROUND,                               /* 7.1 */
};

/*
 * Fills Format: SampleRate, Channels (1 to VD_RING_MAX_CHANNELS, as the core
 * checked), BytesPerSample of SubFormat (float or PCM).
 */
static VOID VdFillFormat(KSDATAFORMAT_WAVEFORMATEXTENSIBLE *Format, UINT32 SampleRate, UINT32 Channels,
                         UINT32 BytesPerSample, const GUID *SubFormat)
{
    UINT32 blockAlign = BytesPerSample * Channels;

    RtlZeroMemory(Format, sizeof *Format);
    Format->DataFormat.FormatSize = sizeof *Format;
    Format->DataFormat.MajorFormat = KSDATAFORMAT_TYPE_AUDIO;
    Format->DataFormat.SubFormat = *SubFormat;
    Format->DataFormat.Specifier = KSDATAFORMAT_SPECIFIER_WAVEFORMATEX;
    Format->WaveFormatExt.Format.wFormatTag = WAVE_FORMAT_EXTENSIBLE;
    Format->WaveFormatExt.Format.nChannels = (WORD)Channels;
    Format->WaveFormatExt.Format.nSamplesPerSec = SampleRate;
    Format->WaveFormatExt.Format.nAvgBytesPerSec = SampleRate * blockAlign;
    Format->WaveFormatExt.Format.nBlockAlign = (WORD)blockAlign;
    Format->WaveFormatExt.Format.wBitsPerSample = (WORD)(8 * BytesPerSample);
    Format->WaveFormatExt.Format.cbSize = sizeof(WAVEFORMATEXTENSIBLE) - sizeof(WAVEFORMATEX);
    Format->WaveFormatExt.Samples.wValidBitsPerSample = (WORD)(8 * BytesPerSample);
    Format->WaveFormatExt.dwChannelMask = VdChannelMasks[Channels];
    Format->WaveFormatExt.SubFormat = *SubFormat;
}

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
    if (DeviceCtx->Channels == 0 || DeviceCtx->Channels > VD_RING_MAX_CHANNELS) {
        return STATUS_INVALID_PARAMETER; /* the core never attaches such a Ring */
    }
    VdFillFormat(&DeviceCtx->FormatFloat, DeviceCtx->SampleRate, DeviceCtx->Channels,
                 VD_FLOAT_BYTES_PER_SAMPLE, &KSDATAFORMAT_SUBTYPE_IEEE_FLOAT);
    VdFillFormat(&DeviceCtx->FormatPcm16, DeviceCtx->SampleRate, DeviceCtx->Channels,
                 VD_PCM16_BYTES_PER_SAMPLE, &KSDATAFORMAT_SUBTYPE_PCM);
    /* Float first, then PCM16: the order the VM endpoint went ACTIVE with. */
    status = VdAddFormat(Device, circuit, formatList, &DeviceCtx->FormatFloat);
    if (NT_SUCCESS(status)) {
        status = VdAddFormat(Device, circuit, formatList, &DeviceCtx->FormatPcm16);
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
