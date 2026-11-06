/*
 * Streaming: Windows' audio engine and the Ring, one copy per packet (ADR 0010).
 *
 * The engine picks the packet count and size. A high-resolution timer fires once
 * per packet: it fills the current capture packet from the Ring, or drains the
 * current render packet into it, then tells ACX the packet is complete. Each pass
 * is scheduled from absolute time with the core clock, so nothing drifts, and the
 * reported position also comes from the core clock (Seam C).
 */
#include "shell.h"

#define VD_MAX_PACKETS 2

typedef struct _VD_STREAM_CONTEXT {
    ACXSTREAM Stream;
    VD_CIRCUIT_DEVICE_CONTEXT *Device;
    WDFTIMER Timer;
    PVOID Packets[VD_MAX_PACKETS];
    ULONG PacketCount;
    ULONG PacketSize;               /* bytes */
    ULONG BytesPerFrame;            /* from the stream's format */
    BOOLEAN Pcm16;                  /* 16-bit PCM, converted in the copy; else float, copied (ADR 0005) */
    ULONG FirstPacketOffset;
    ULONGLONG TickHz;
    volatile BOOLEAN Running;       /* cleared before the timer stops: passes only re-arm while set */
    ULONGLONG RunStartTicks;        /* QPC when the stream last started running */
    ULONGLONG RunStartFrames;       /* position then */
    ULONGLONG PausedFrames;         /* position while not running */
    ULONGLONG PacketsSinceRun;
    volatile LONG CurrentPacket;
    volatile LONG64 LastPacketStart;
    volatile LONG64 CurrentPacketStart;
} VD_STREAM_CONTEXT;
WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(VD_STREAM_CONTEXT, VdStreamContext)

typedef struct _VD_TIMER_CONTEXT {
    VD_STREAM_CONTEXT *StreamCtx;
} VD_TIMER_CONTEXT;
WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(VD_TIMER_CONTEXT, VdTimerContext)

static EVT_ACX_STREAM_PREPARE_HARDWARE VdEvtStreamPrepareHardware;
static EVT_ACX_STREAM_RELEASE_HARDWARE VdEvtStreamReleaseHardware;
static EVT_ACX_STREAM_RUN VdEvtStreamRun;
static EVT_ACX_STREAM_PAUSE VdEvtStreamPause;
static EVT_ACX_STREAM_GET_HW_LATENCY VdEvtStreamGetHwLatency;
static EVT_ACX_STREAM_ALLOCATE_RTPACKETS VdEvtStreamAllocateRtPackets;
static EVT_ACX_STREAM_FREE_RTPACKETS VdEvtStreamFreeRtPackets;
static EVT_ACX_STREAM_GET_CAPTURE_PACKET VdEvtStreamGetCapturePacket;
static EVT_ACX_STREAM_SET_RENDER_PACKET VdEvtStreamSetRenderPacket;
static EVT_ACX_STREAM_GET_CURRENT_PACKET VdEvtStreamGetCurrentPacket;
static EVT_ACX_STREAM_GET_PRESENTATION_POSITION VdEvtStreamGetPresentationPosition;
static EVT_WDF_TIMER VdEvtStreamTimer;

static ULONGLONG VdNow(void)
{
    return (ULONGLONG)KeQueryPerformanceCounter(NULL).QuadPart;
}

static ULONGLONG VdPosition(const VD_STREAM_CONTEXT *StreamCtx)
{
    if (!StreamCtx->Running) {
        return StreamCtx->PausedFrames;
    }
    return StreamCtx->RunStartFrames +
           vd_clock_frames(StreamCtx->RunStartTicks, VdNow(), StreamCtx->TickHz, VD_SAMPLE_RATE);
}

/* Arms the timer for the end of the next packet, measured from when the stream started running. */
static VOID VdScheduleNextPass(VD_STREAM_CONTEXT *StreamCtx)
{
    ULONGLONG packetFrames = StreamCtx->PacketSize / StreamCtx->BytesPerFrame;
    ULONGLONG due = StreamCtx->RunStartTicks +
                    vd_clock_ticks_for((StreamCtx->PacketsSinceRun + 1) * packetFrames,
                                       StreamCtx->TickHz, VD_SAMPLE_RATE);
    ULONGLONG now = VdNow();
    LONGLONG wait = -1; /* due already: fire as soon as possible */

    if (due > now) {
        /* Timer waits are in 100 ns units: the core clock converts ticks at 10 MHz. */
        wait = -(LONGLONG)vd_clock_frames(now, due, StreamCtx->TickHz, 10000000u);
        if (wait == 0) {
            wait = -1;
        }
    }
    (void)WdfTimerStart(StreamCtx->Timer, wait);
}

static VOID VdEvtStreamTimer(WDFTIMER Timer)
{
    VD_STREAM_CONTEXT *streamCtx = VdTimerContext(Timer)->StreamCtx;
    VD_CIRCUIT_DEVICE_CONTEXT *device = streamCtx->Device;
    ULONG index = (ULONG)streamCtx->CurrentPacket % streamCtx->PacketCount;
    UCHAR *packet = (UCHAR *)streamCtx->Packets[index] + (index == 0 ? streamCtx->FirstPacketOffset : 0);
    uint32_t frames = streamCtx->PacketSize / streamCtx->BytesPerFrame;
    ULONGLONG completedAt;
    LONG completed;

    if (ExAcquireRundownProtection(&device->Rundown)) {
        if (device->Kind == VD_KIND_MICROPHONE) {
            if (streamCtx->Pcm16) {
                vd_ring_read_pcm16(device->Ring, (int16_t *)packet, frames);
            } else {
                vd_ring_read(device->Ring, (float *)packet, frames);
            }
        } else if (streamCtx->Pcm16) {
            vd_ring_write_pcm16(device->Ring, (const int16_t *)packet, frames);
        } else {
            vd_ring_write(device->Ring, (const float *)packet, frames);
        }
        ExReleaseRundownProtection(&device->Rundown);
    } else if (device->Kind == VD_KIND_MICROPHONE) {
        RtlZeroMemory(packet, streamCtx->PacketSize); /* Device removed: silence */
    }

    completed = InterlockedIncrement(&streamCtx->CurrentPacket) - 1;
    completedAt = VdNow();
    InterlockedExchange64(&streamCtx->LastPacketStart, streamCtx->CurrentPacketStart);
    InterlockedExchange64(&streamCtx->CurrentPacketStart, (LONG64)completedAt);
    (void)AcxRtStreamNotifyPacketComplete(streamCtx->Stream, (ULONGLONG)completed, completedAt);

    streamCtx->PacketsSinceRun++;
    if (streamCtx->Running) {
        VdScheduleNextPass(streamCtx);
    }
}

NTSTATUS VdEvtCircuitCreateStream(WDFDEVICE Device, ACXCIRCUIT Circuit, ACXPIN Pin,
                                  PACXSTREAM_INIT StreamInit, ACXDATAFORMAT StreamFormat,
                                  const GUID *SignalProcessingMode, ACXOBJECTBAG VarArguments)
{
    ACX_STREAM_CALLBACKS streamCallbacks;
    ACX_RT_STREAM_CALLBACKS rtCallbacks;
    WDF_OBJECT_ATTRIBUTES attributes;
    VD_STREAM_CONTEXT *streamCtx;
    LARGE_INTEGER frequency;
    ACXSTREAM stream;
    ULONG bytesPerFrame = AcxDataFormatGetBlockAlign(StreamFormat);
    ULONG bitsPerSample = AcxDataFormatGetBitsPerSample(StreamFormat);
    BOOLEAN pcm16 = bitsPerSample == 16 && bytesPerFrame == VD_PCM16_BYTES_PER_FRAME;
    BOOLEAN ringFloat = bitsPerSample == 32 && bytesPerFrame == VD_FLOAT_BYTES_PER_FRAME;
    NTSTATUS status;

    UNREFERENCED_PARAMETER(Pin);
    UNREFERENCED_PARAMETER(SignalProcessingMode);
    UNREFERENCED_PARAMETER(VarArguments);

    /* Only the host pin's two mono formats (circuit.c); refuse before anything is created. */
    if (!pcm16 && !ringFloat) {
        return STATUS_NOT_SUPPORTED;
    }

    ACX_STREAM_CALLBACKS_INIT(&streamCallbacks);
    streamCallbacks.EvtAcxStreamPrepareHardware = VdEvtStreamPrepareHardware;
    streamCallbacks.EvtAcxStreamReleaseHardware = VdEvtStreamReleaseHardware;
    streamCallbacks.EvtAcxStreamRun = VdEvtStreamRun;
    streamCallbacks.EvtAcxStreamPause = VdEvtStreamPause;
    status = AcxStreamInitAssignAcxStreamCallbacks(StreamInit, &streamCallbacks);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    ACX_RT_STREAM_CALLBACKS_INIT(&rtCallbacks);
    rtCallbacks.EvtAcxStreamGetHwLatency = VdEvtStreamGetHwLatency;
    rtCallbacks.EvtAcxStreamAllocateRtPackets = VdEvtStreamAllocateRtPackets;
    rtCallbacks.EvtAcxStreamFreeRtPackets = VdEvtStreamFreeRtPackets;
    rtCallbacks.EvtAcxStreamGetCurrentPacket = VdEvtStreamGetCurrentPacket;
    rtCallbacks.EvtAcxStreamGetPresentationPosition = VdEvtStreamGetPresentationPosition;
    if (VdCircuitDeviceContext(Device)->Kind == VD_KIND_MICROPHONE) {
        rtCallbacks.EvtAcxStreamGetCapturePacket = VdEvtStreamGetCapturePacket;
    } else {
        rtCallbacks.EvtAcxStreamSetRenderPacket = VdEvtStreamSetRenderPacket;
    }
    status = AcxStreamInitAssignAcxRtStreamCallbacks(StreamInit, &rtCallbacks);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    AcxStreamInitSetAcxRtStreamSupportsNotifications(StreamInit);

    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, VD_STREAM_CONTEXT);
    status = AcxRtStreamCreate(Device, Circuit, &attributes, &StreamInit, &stream);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    streamCtx = VdStreamContext(stream);
    streamCtx->Stream = stream;
    streamCtx->Device = VdCircuitDeviceContext(Device);
    streamCtx->BytesPerFrame = bytesPerFrame;
    streamCtx->Pcm16 = pcm16;
    KeQueryPerformanceCounter(&frequency);
    streamCtx->TickHz = (ULONGLONG)frequency.QuadPart;
    return STATUS_SUCCESS;
}

static NTSTATUS VdEvtStreamPrepareHardware(ACXSTREAM Stream)
{
    VD_STREAM_CONTEXT *streamCtx = VdStreamContext(Stream);
    WDF_TIMER_CONFIG timerConfig;
    WDF_OBJECT_ATTRIBUTES attributes;
    NTSTATUS status;

    if (streamCtx->Timer != NULL) {
        return STATUS_SUCCESS;
    }
    WDF_TIMER_CONFIG_INIT(&timerConfig, VdEvtStreamTimer);
    timerConfig.UseHighResolutionTimer = WdfTrue;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, VD_TIMER_CONTEXT);
    attributes.ParentObject = Stream;
    status = WdfTimerCreate(&timerConfig, &attributes, &streamCtx->Timer);
    if (NT_SUCCESS(status)) {
        VdTimerContext(streamCtx->Timer)->StreamCtx = streamCtx;
    }
    return status;
}

/* Pause to Stop, as Microsoft's ACX stream engine does it: the next start begins at packet 0. */
static NTSTATUS VdEvtStreamReleaseHardware(ACXSTREAM Stream)
{
    VD_STREAM_CONTEXT *streamCtx = VdStreamContext(Stream);

    streamCtx->Running = FALSE;
    if (streamCtx->Timer != NULL) {
        (void)WdfTimerStop(streamCtx->Timer, TRUE);
        WdfObjectDelete(streamCtx->Timer);
        streamCtx->Timer = NULL;
    }
    KeFlushQueuedDpcs();

    streamCtx->PausedFrames = 0;
    streamCtx->PacketsSinceRun = 0;
    InterlockedExchange(&streamCtx->CurrentPacket, 0);
    InterlockedExchange64(&streamCtx->LastPacketStart, 0);
    InterlockedExchange64(&streamCtx->CurrentPacketStart, 0);
    return STATUS_SUCCESS;
}

static NTSTATUS VdEvtStreamRun(ACXSTREAM Stream)
{
    VD_STREAM_CONTEXT *streamCtx = VdStreamContext(Stream);

    if (streamCtx->Running) {
        return STATUS_SUCCESS;
    }
    if (streamCtx->Timer == NULL || streamCtx->PacketCount == 0) {
        return STATUS_INVALID_DEVICE_STATE;
    }
    streamCtx->RunStartTicks = VdNow();
    streamCtx->RunStartFrames = streamCtx->PausedFrames;
    streamCtx->PacketsSinceRun = 0;
    InterlockedExchange64(&streamCtx->CurrentPacketStart, (LONG64)streamCtx->RunStartTicks);
    streamCtx->Running = TRUE;
    VdScheduleNextPass(streamCtx);
    return STATUS_SUCCESS;
}

static NTSTATUS VdEvtStreamPause(ACXSTREAM Stream)
{
    VD_STREAM_CONTEXT *streamCtx = VdStreamContext(Stream);

    if (!streamCtx->Running) {
        return STATUS_SUCCESS;
    }
    {
        ULONGLONG frames = VdPosition(streamCtx);

        streamCtx->Running = FALSE;                 /* an in-flight pass won't re-arm */
        (void)WdfTimerStop(streamCtx->Timer, TRUE); /* waits for that pass */
        streamCtx->PausedFrames = frames;
    }
    return STATUS_SUCCESS;
}

static NTSTATUS VdEvtStreamGetHwLatency(ACXSTREAM Stream, ULONG *FifoSize, ULONG *Delay)
{
    UNREFERENCED_PARAMETER(Stream);
    *FifoSize = 0; /* no hardware between the packets and the Ring */
    *Delay = 0;
    return STATUS_SUCCESS;
}

static VOID VdFreePacketBuffers(VD_STREAM_CONTEXT *StreamCtx)
{
    for (ULONG i = 0; i < VD_MAX_PACKETS; i++) {
        if (StreamCtx->Packets[i] != NULL) {
            ExFreePoolWithTag(StreamCtx->Packets[i], VD_POOL_TAG);
            StreamCtx->Packets[i] = NULL;
        }
    }
    StreamCtx->PacketCount = 0;
}

static VOID VdEvtStreamFreeRtPackets(ACXSTREAM Stream, PACX_RTPACKET Packets, ULONG PacketCount)
{
    for (ULONG i = 0; i < PacketCount; i++) {
        if (Packets[i].RtPacketBuffer.u.MdlType.Mdl != NULL) {
            IoFreeMdl(Packets[i].RtPacketBuffer.u.MdlType.Mdl);
        }
    }
    ExFreePoolWithTag(Packets, VD_POOL_TAG);
    VdFreePacketBuffers(VdStreamContext(Stream));
}

/*
 * Page-aligned, zeroed packets (as in Microsoft's ACX sample): packet 0 ends on
 * a page boundary and packet 1 starts on one, so no other kernel memory is ever
 * mapped into the audio engine.
 */
static NTSTATUS VdEvtStreamAllocateRtPackets(ACXSTREAM Stream, ULONG PacketCount, ULONG PacketSize,
                                             PACX_RTPACKET *Packets)
{
    VD_STREAM_CONTEXT *streamCtx = VdStreamContext(Stream);
    PACX_RTPACKET packets;
    ULONG allocation;
    NTSTATUS status;

    *Packets = NULL;
    if (PacketCount == 0 || PacketCount > VD_MAX_PACKETS || PacketSize == 0 ||
        PacketSize % streamCtx->BytesPerFrame != 0) {
        return STATUS_INVALID_PARAMETER;
    }
    status = RtlULongAdd(PacketSize, PAGE_SIZE - 1, &allocation);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    allocation = (allocation / PAGE_SIZE) * PAGE_SIZE;

    packets = (PACX_RTPACKET)ExAllocatePool2(POOL_FLAG_NON_PAGED, sizeof(ACX_RTPACKET) * PacketCount,
                                             VD_POOL_TAG);
    if (packets == NULL) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }

    for (ULONG i = 0; i < PacketCount; i++) {
        PMDL mdl;

        ACX_RTPACKET_INIT(&packets[i]);
        streamCtx->Packets[i] = ExAllocatePool2(POOL_FLAG_NON_PAGED, allocation, VD_POOL_TAG);
        if (streamCtx->Packets[i] == NULL) {
            status = STATUS_INSUFFICIENT_RESOURCES;
            break;
        }
        mdl = IoAllocateMdl(streamCtx->Packets[i], allocation, FALSE, TRUE, NULL);
        if (mdl == NULL) {
            status = STATUS_INSUFFICIENT_RESOURCES;
            break;
        }
        MmBuildMdlForNonPagedPool(mdl);
        WDF_MEMORY_DESCRIPTOR_INIT_MDL(&packets[i].RtPacketBuffer, mdl, allocation);
        packets[i].RtPacketSize = PacketSize;
        packets[i].RtPacketOffset = i == 0 ? allocation - PacketSize : 0;
    }
    if (!NT_SUCCESS(status)) {
        VdEvtStreamFreeRtPackets(Stream, packets, PacketCount);
        return status;
    }

    streamCtx->PacketCount = PacketCount;
    streamCtx->PacketSize = PacketSize;
    streamCtx->FirstPacketOffset = allocation - PacketSize;
    *Packets = packets;
    return STATUS_SUCCESS;
}

static NTSTATUS VdEvtStreamGetCurrentPacket(ACXSTREAM Stream, PULONG CurrentPacket)
{
    *CurrentPacket = (ULONG)InterlockedCompareExchange(&VdStreamContext(Stream)->CurrentPacket, 0, 0);
    return STATUS_SUCCESS;
}

static NTSTATUS VdEvtStreamGetCapturePacket(ACXSTREAM Stream, PULONG LastCapturePacket,
                                            PULONGLONG QPCPacketStart, PBOOLEAN MoreData)
{
    VD_STREAM_CONTEXT *streamCtx = VdStreamContext(Stream);

    *LastCapturePacket = (ULONG)InterlockedCompareExchange(&streamCtx->CurrentPacket, 0, 0) - 1;
    *QPCPacketStart = (ULONGLONG)InterlockedCompareExchange64(&streamCtx->LastPacketStart, 0, 0);
    *MoreData = FALSE;
    return STATUS_SUCCESS;
}

/* The engine must fill exactly the next packet: anything else is late or early. */
static NTSTATUS VdEvtStreamSetRenderPacket(ACXSTREAM Stream, ULONG Packet, ULONG Flags,
                                           ULONG EosPacketLength)
{
    ULONG current = (ULONG)InterlockedCompareExchange(&VdStreamContext(Stream)->CurrentPacket, 0, 0);

    UNREFERENCED_PARAMETER(Flags);
    UNREFERENCED_PARAMETER(EosPacketLength);

    if (Packet <= current) {
        return STATUS_DATA_LATE_ERROR;
    }
    if (Packet > current + 1) {
        return STATUS_DATA_OVERRUN;
    }
    return STATUS_SUCCESS;
}

static NTSTATUS VdEvtStreamGetPresentationPosition(ACXSTREAM Stream, PULONGLONG PositionInBlocks,
                                                   PULONGLONG QPCPosition)
{
    VD_STREAM_CONTEXT *streamCtx = VdStreamContext(Stream);

    *QPCPosition = VdNow();
    *PositionInBlocks = VdPosition(streamCtx); /* one block is one mono frame */
    return STATUS_SUCCESS;
}
