/*
 * Streaming: Windows' audio engine and the Ring, one copy per packet (ADR 0010).
 *
 * The engine picks the packet count and size. A high-resolution timer fires once
 * per packet: it fills the current capture packet from the Ring, or drains the
 * current render packet into it, then tells ACX the packet is complete. Each pass
 * is scheduled from absolute time with the core clock, so nothing drifts, and the
 * reported position also comes from the core clock (pattie_clock_*).
 */
#include "shell.h"

#define MAX_PACKETS 2

typedef struct _STREAM_CONTEXT {
    ACXSTREAM Stream;
    CIRCUIT_DEVICE_CONTEXT *Device;
    WDFTIMER Timer;
    PVOID Packets[MAX_PACKETS];
    ULONG PacketCount;
    ULONG PacketSize;               /* bytes */
    ULONG BytesPerFrame;            /* from the stream's format */
    ULONG SampleRate; /* the Device's, which the stream's format matches (ADR 0023) */
    BOOLEAN Pcm16; /* 16-bit PCM, converted in the copy; else float, copied (ADR 0005) */
    ULONG FirstPacketOffset;
    ULONGLONG TickHz;
    volatile BOOLEAN Running; /* cleared before the timer stops: passes only re-arm while set */
    ULONGLONG RunStartTicks; /* QPC when the stream last started running; moved past a stall */
    ULONGLONG RunStartFrames; /* position then */
    ULONGLONG PausedFrames; /* position while not running */
    ULONGLONG PacketsSinceRun;
    volatile LONG CurrentPacket;
    volatile LONG64 LastPacketStart;
    volatile LONG64 CurrentPacketStart;
    /*
     * Timing since the stream last started running, reported in the StreamStopped
     * event (trace.h). Written by the timer pass and Run only; read after the timer
     * is stopped and waited for.
     */
    ULONGLONG DueTicks; /* when the armed pass should fire */
    ULONGLONG Passes;
    ULONGLONG LatePasses; /* armed when already due: the timer fell behind */
    ULONGLONG Stalls; /* so far behind the schedule skipped ahead (pattie_clock_next_pass) */
    ULONGLONG MaxLatenessTicks; /* worst pass: fired this long after it was due */
    ULONGLONG TotalLatenessTicks;
} STREAM_CONTEXT;
WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(STREAM_CONTEXT, StreamContext)

typedef struct _TIMER_CONTEXT {
    STREAM_CONTEXT *StreamCtx;
} TIMER_CONTEXT;
WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(TIMER_CONTEXT, TimerContext)

static EVT_ACX_STREAM_PREPARE_HARDWARE EvtStreamPrepareHardware;
static EVT_ACX_STREAM_RELEASE_HARDWARE EvtStreamReleaseHardware;
static EVT_ACX_STREAM_RUN EvtStreamRun;
static EVT_ACX_STREAM_PAUSE EvtStreamPause;
static EVT_ACX_STREAM_GET_HW_LATENCY EvtStreamGetHwLatency;
static EVT_ACX_STREAM_ALLOCATE_RTPACKETS EvtStreamAllocateRtPackets;
static EVT_ACX_STREAM_FREE_RTPACKETS EvtStreamFreeRtPackets;
static EVT_ACX_STREAM_GET_CAPTURE_PACKET EvtStreamGetCapturePacket;
static EVT_ACX_STREAM_SET_RENDER_PACKET EvtStreamSetRenderPacket;
static EVT_ACX_STREAM_GET_CURRENT_PACKET EvtStreamGetCurrentPacket;
static EVT_ACX_STREAM_GET_PRESENTATION_POSITION EvtStreamGetPresentationPosition;
static EVT_WDF_TIMER EvtStreamTimer;

static ULONGLONG Now(void)
{
    return (ULONGLONG)KeQueryPerformanceCounter(NULL).QuadPart;
}

/* Ticks as microseconds, with the core clock's overflow-safe conversion. */
static ULONGLONG TicksToUs(const STREAM_CONTEXT *StreamCtx, ULONGLONG Ticks)
{
    return pattie_clock_frames(0, Ticks, StreamCtx->TickHz, 1000000u);
}

/*
 * The StreamStopped event: how the timer kept up since the stream last ran, plus
 * the Ring's Gap and Overflow totals. Called with the timer stopped.
 */
static VOID TraceStreamStopped(const STREAM_CONTEXT *StreamCtx, const char *Reason)
{
    CIRCUIT_DEVICE_CONTEXT *device = StreamCtx->Device;
    uint64_t gapFrames = 0;
    uint64_t overflowFrames = 0;
    ULONGLONG averageLatenessTicks =
        StreamCtx->Passes == 0 ? 0 : StreamCtx->TotalLatenessTicks / StreamCtx->Passes;

    if (ExAcquireRundownProtection(&device->Rundown)) {
        pattie_ring_losses(device->Ring, &gapFrames, &overflowFrames);
        ExReleaseRundownProtection(&device->Rundown);
    }
    TraceLoggingWrite(
        TraceProvider, "StreamStopped", TraceLoggingLevel(TRACE_LEVEL_INFO),
        TraceLoggingUInt32(device->DeviceId, "DeviceId"), TraceLoggingString(Reason, "Reason"),
        TraceLoggingUInt64(StreamCtx->Passes, "Passes"),
        TraceLoggingUInt64(StreamCtx->LatePasses, "LatePasses"),
        TraceLoggingUInt64(StreamCtx->Stalls, "Stalls"),
        TraceLoggingUInt64(TicksToUs(StreamCtx, StreamCtx->MaxLatenessTicks), "MaxLatenessUs"),
        TraceLoggingUInt64(TicksToUs(StreamCtx, averageLatenessTicks), "AvgLatenessUs"),
        TraceLoggingUInt64(gapFrames, "RingGapFrames"),
        TraceLoggingUInt64(overflowFrames, "RingOverflowFrames"));
}

static ULONGLONG Position(const STREAM_CONTEXT *StreamCtx)
{
    if (!StreamCtx->Running) {
        return StreamCtx->PausedFrames;
    }
    return StreamCtx->RunStartFrames + pattie_clock_frames(StreamCtx->RunStartTicks, Now(),
                                                           StreamCtx->TickHz,
                                                           StreamCtx->SampleRate);
}

/*
 * Arms the timer for the end of the next packet, measured from when the stream
 * started running. A pass or two behind fires at once to catch up; after a stall
 * (more passes behind than the engine's buffer holds) the core moves the start
 * forward instead, so a sleep or a suspended VM doesn't set off a burst of
 * passes, one per missed packet (as Microsoft's ACX stream engine does).
 */
static VOID ScheduleNextPass(STREAM_CONTEXT *StreamCtx)
{
    uint64_t start = StreamCtx->RunStartTicks;
    uint64_t due = 0;
    ULONGLONG now = Now();
    LONGLONG wait = -1; /* due already: fire as soon as possible */

    if (pattie_clock_next_pass(
            &start, StreamCtx->PacketsSinceRun, StreamCtx->PacketSize / StreamCtx->BytesPerFrame,
            StreamCtx->PacketCount, now, StreamCtx->TickHz, StreamCtx->SampleRate, &due)) {
        StreamCtx->RunStartTicks = start;
        StreamCtx->Stalls++;
    }
    StreamCtx->DueTicks = due;
    if (due <= now) {
        StreamCtx->LatePasses++;
    } else {
        /* Timer waits are in 100 ns units: the core clock converts ticks at 10 MHz. */
        wait = -(LONGLONG)pattie_clock_frames(now, due, StreamCtx->TickHz, 10000000u);
        if (wait == 0) {
            wait = -1;
        }
    }
    (void)WdfTimerStart(StreamCtx->Timer, wait);
}

static VOID EvtStreamTimer(WDFTIMER Timer)
{
    STREAM_CONTEXT *streamCtx = TimerContext(Timer)->StreamCtx;
    CIRCUIT_DEVICE_CONTEXT *device = streamCtx->Device;
    ULONG index = (ULONG)streamCtx->CurrentPacket % streamCtx->PacketCount;
    UCHAR *packet =
        (UCHAR *)streamCtx->Packets[index] + (index == 0 ? streamCtx->FirstPacketOffset : 0);
    uint32_t frames = streamCtx->PacketSize / streamCtx->BytesPerFrame;
    ULONGLONG firedAt = Now();
    ULONGLONG completedAt;
    LONG completed;

    if (firedAt > streamCtx->DueTicks) {
        ULONGLONG lateness = firedAt - streamCtx->DueTicks;

        streamCtx->TotalLatenessTicks += lateness;
        if (lateness > streamCtx->MaxLatenessTicks) {
            streamCtx->MaxLatenessTicks = lateness;
        }
    }
    streamCtx->Passes++;

    if (ExAcquireRundownProtection(&device->Rundown)) {
        if (device->Kind == PATTIE_KIND_MICROPHONE) {
            if (streamCtx->Pcm16) {
                pattie_ring_read_pcm16(device->Ring, (int16_t *)packet, frames);
            } else {
                pattie_ring_read(device->Ring, (float *)packet, frames);
            }
        } else if (streamCtx->Pcm16) {
            pattie_ring_write_pcm16(device->Ring, (const int16_t *)packet, frames);
        } else {
            pattie_ring_write(device->Ring, (const float *)packet, frames);
        }
        /* The clock pair (ADR 0022): how far this side has got, and when, in
         * nanoseconds of the performance counter, as apps read it. */
        if (device->Kind == PATTIE_KIND_MICROPHONE) {
            pattie_ring_publish_read_clock(device->Ring, pattie_clock_ns(Now(), streamCtx->TickHz));
        } else {
            pattie_ring_publish_write_clock(device->Ring,
                                            pattie_clock_ns(Now(), streamCtx->TickHz));
        }
        /* Wake the app's paced writer, reader or playout (ADR 0021). */
        (void)KeSetEvent(device->WakeEvent, IO_NO_INCREMENT, FALSE);
        ExReleaseRundownProtection(&device->Rundown);
    } else if (device->Kind == PATTIE_KIND_MICROPHONE) {
        RtlZeroMemory(packet, streamCtx->PacketSize); /* Device removed: silence */
    }

    completed = InterlockedIncrement(&streamCtx->CurrentPacket) - 1;
    completedAt = Now();
    InterlockedExchange64(&streamCtx->LastPacketStart, streamCtx->CurrentPacketStart);
    InterlockedExchange64(&streamCtx->CurrentPacketStart, (LONG64)completedAt);
    (void)AcxRtStreamNotifyPacketComplete(streamCtx->Stream, (ULONGLONG)completed, completedAt);

    streamCtx->PacketsSinceRun++;
    if (streamCtx->Running) {
        ScheduleNextPass(streamCtx);
    }
}

NTSTATUS EvtCircuitCreateStream(WDFDEVICE Device, ACXCIRCUIT Circuit, ACXPIN Pin,
                                PACXSTREAM_INIT StreamInit, ACXDATAFORMAT StreamFormat,
                                const GUID *SignalProcessingMode, ACXOBJECTBAG VarArguments)
{
    ACX_STREAM_CALLBACKS streamCallbacks;
    ACX_RT_STREAM_CALLBACKS rtCallbacks;
    WDF_OBJECT_ATTRIBUTES attributes;
    STREAM_CONTEXT *streamCtx;
    LARGE_INTEGER frequency;
    ACXSTREAM stream;
    CIRCUIT_DEVICE_CONTEXT *device = CircuitDeviceContext(Device);
    ULONG bytesPerFrame = AcxDataFormatGetBlockAlign(StreamFormat);
    ULONG bitsPerSample = AcxDataFormatGetBitsPerSample(StreamFormat);
    ULONG sampleRate = AcxDataFormatGetSampleRate(StreamFormat);
    ULONG channels = AcxDataFormatGetChannelsCount(StreamFormat);
    BOOLEAN deviceShape = sampleRate == device->SampleRate && channels == device->Channels;
    BOOLEAN pcm16 =
        deviceShape && bitsPerSample == 16 && bytesPerFrame == PCM16_BYTES_PER_SAMPLE * channels;
    BOOLEAN ringFloat =
        deviceShape && bitsPerSample == 32 && bytesPerFrame == FLOAT_BYTES_PER_SAMPLE * channels;
    NTSTATUS status;

    UNREFERENCED_PARAMETER(Pin);
    UNREFERENCED_PARAMETER(SignalProcessingMode);
    UNREFERENCED_PARAMETER(VarArguments);

    /*
     * Only the host pin's two formats, at the Device's rate and channels
     * (circuit.c); refuse before anything is created.
     */
    if (!pcm16 && !ringFloat) {
        return STATUS_NOT_SUPPORTED;
    }

    ACX_STREAM_CALLBACKS_INIT(&streamCallbacks);
    streamCallbacks.EvtAcxStreamPrepareHardware = EvtStreamPrepareHardware;
    streamCallbacks.EvtAcxStreamReleaseHardware = EvtStreamReleaseHardware;
    streamCallbacks.EvtAcxStreamRun = EvtStreamRun;
    streamCallbacks.EvtAcxStreamPause = EvtStreamPause;
    status = AcxStreamInitAssignAcxStreamCallbacks(StreamInit, &streamCallbacks);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    ACX_RT_STREAM_CALLBACKS_INIT(&rtCallbacks);
    rtCallbacks.EvtAcxStreamGetHwLatency = EvtStreamGetHwLatency;
    rtCallbacks.EvtAcxStreamAllocateRtPackets = EvtStreamAllocateRtPackets;
    rtCallbacks.EvtAcxStreamFreeRtPackets = EvtStreamFreeRtPackets;
    rtCallbacks.EvtAcxStreamGetCurrentPacket = EvtStreamGetCurrentPacket;
    rtCallbacks.EvtAcxStreamGetPresentationPosition = EvtStreamGetPresentationPosition;
    if (device->Kind == PATTIE_KIND_MICROPHONE) {
        rtCallbacks.EvtAcxStreamGetCapturePacket = EvtStreamGetCapturePacket;
    } else {
        rtCallbacks.EvtAcxStreamSetRenderPacket = EvtStreamSetRenderPacket;
    }
    status = AcxStreamInitAssignAcxRtStreamCallbacks(StreamInit, &rtCallbacks);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    AcxStreamInitSetAcxRtStreamSupportsNotifications(StreamInit);

    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, STREAM_CONTEXT);
    status = AcxRtStreamCreate(Device, Circuit, &attributes, &StreamInit, &stream);
    if (!NT_SUCCESS(status)) {
        return status;
    }

    streamCtx = StreamContext(stream);
    streamCtx->Stream = stream;
    streamCtx->Device = device;
    streamCtx->BytesPerFrame = bytesPerFrame;
    streamCtx->SampleRate = sampleRate;
    streamCtx->Pcm16 = pcm16;
    KeQueryPerformanceCounter(&frequency);
    streamCtx->TickHz = (ULONGLONG)frequency.QuadPart;

    /* Which format Windows opened the stream in: float, or PCM16 through the conversion. */
    TraceLoggingWrite(
        TraceProvider, "StreamCreated", TraceLoggingLevel(TRACE_LEVEL_INFO),
        TraceLoggingUInt32(streamCtx->Device->DeviceId, "DeviceId"),
        TraceLoggingUInt32(streamCtx->Device->Kind, "Kind"), TraceLoggingBoolean(pcm16, "Pcm16"),
        TraceLoggingUInt32(bitsPerSample, "BitsPerSample"),
        TraceLoggingUInt32(bytesPerFrame, "BlockAlign"),
        TraceLoggingUInt32(sampleRate, "SampleRate"), TraceLoggingUInt32(channels, "Channels"));
    return STATUS_SUCCESS;
}

static NTSTATUS EvtStreamPrepareHardware(ACXSTREAM Stream)
{
    STREAM_CONTEXT *streamCtx = StreamContext(Stream);
    WDF_TIMER_CONFIG timerConfig;
    WDF_OBJECT_ATTRIBUTES attributes;
    NTSTATUS status;

    if (streamCtx->Timer != NULL) {
        return STATUS_SUCCESS;
    }
    WDF_TIMER_CONFIG_INIT(&timerConfig, EvtStreamTimer);
    timerConfig.UseHighResolutionTimer = WdfTrue;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, TIMER_CONTEXT);
    attributes.ParentObject = Stream;
    status = WdfTimerCreate(&timerConfig, &attributes, &streamCtx->Timer);
    if (NT_SUCCESS(status)) {
        TimerContext(streamCtx->Timer)->StreamCtx = streamCtx;
    }
    return status;
}

/* Pause to Stop, as Microsoft's ACX stream engine does it: the next start begins at packet 0. */
static NTSTATUS EvtStreamReleaseHardware(ACXSTREAM Stream)
{
    STREAM_CONTEXT *streamCtx = StreamContext(Stream);
    BOOLEAN wasRunning = streamCtx->Running;

    streamCtx->Running = FALSE;
    if (streamCtx->Timer != NULL) {
        (void)WdfTimerStop(streamCtx->Timer, TRUE);
        WdfObjectDelete(streamCtx->Timer);
        streamCtx->Timer = NULL;
    }
    KeFlushQueuedDpcs();
    if (wasRunning) {
        TraceStreamStopped(streamCtx, "Release"); /* a Pause already reported otherwise */
    }

    streamCtx->PausedFrames = 0;
    streamCtx->PacketsSinceRun = 0;
    InterlockedExchange(&streamCtx->CurrentPacket, 0);
    InterlockedExchange64(&streamCtx->LastPacketStart, 0);
    InterlockedExchange64(&streamCtx->CurrentPacketStart, 0);
    return STATUS_SUCCESS;
}

/*
 * A Microphone starting to record: drop audio the app wrote while nobody was
 * recording, keeping one packet, so the first packet is fresh (ADR 0014). Returns
 * the frames dropped, for the StreamRun event.
 */
static ULONGLONG SkipStaleCapture(STREAM_CONTEXT *StreamCtx)
{
    CIRCUIT_DEVICE_CONTEXT *device = StreamCtx->Device;
    ULONGLONG skipped = 0;

    if (device->Kind != PATTIE_KIND_MICROPHONE || !ExAcquireRundownProtection(&device->Rundown)) {
        return 0;
    }
    skipped =
        pattie_ring_skip_stale(device->Ring, StreamCtx->PacketSize / StreamCtx->BytesPerFrame);
    ExReleaseRundownProtection(&device->Rundown);
    return skipped;
}

static NTSTATUS EvtStreamRun(ACXSTREAM Stream)
{
    STREAM_CONTEXT *streamCtx = StreamContext(Stream);
    ULONGLONG skippedFrames;

    if (streamCtx->Running) {
        return STATUS_SUCCESS;
    }
    if (streamCtx->Timer == NULL || streamCtx->PacketCount == 0) {
        return STATUS_INVALID_DEVICE_STATE;
    }
    streamCtx->RunStartTicks = Now();
    streamCtx->RunStartFrames = streamCtx->PausedFrames;
    streamCtx->PacketsSinceRun = 0;
    streamCtx->Passes = 0;
    streamCtx->LatePasses = 0;
    streamCtx->Stalls = 0;
    streamCtx->MaxLatenessTicks = 0;
    streamCtx->TotalLatenessTicks = 0;
    skippedFrames = SkipStaleCapture(streamCtx); /* before the timer is armed */
    TraceLoggingWrite(TraceProvider, "StreamRun", TraceLoggingLevel(TRACE_LEVEL_INFO),
                      TraceLoggingUInt32(streamCtx->Device->DeviceId, "DeviceId"),
                      TraceLoggingUInt64(skippedFrames, "StaleFramesSkipped"));
    InterlockedExchange64(&streamCtx->CurrentPacketStart, (LONG64)streamCtx->RunStartTicks);
    streamCtx->Running = TRUE;
    ScheduleNextPass(streamCtx);
    return STATUS_SUCCESS;
}

static NTSTATUS EvtStreamPause(ACXSTREAM Stream)
{
    STREAM_CONTEXT *streamCtx = StreamContext(Stream);

    if (!streamCtx->Running) {
        return STATUS_SUCCESS;
    }
    {
        ULONGLONG frames = Position(streamCtx);

        streamCtx->Running = FALSE; /* an in-flight pass won't re-arm */
        (void)WdfTimerStop(streamCtx->Timer, TRUE); /* waits for that pass */
        streamCtx->PausedFrames = frames;
    }
    TraceStreamStopped(streamCtx, "Pause");
    return STATUS_SUCCESS;
}

static NTSTATUS EvtStreamGetHwLatency(ACXSTREAM Stream, ULONG *FifoSize, ULONG *Delay)
{
    UNREFERENCED_PARAMETER(Stream);
    *FifoSize = 0; /* no hardware between the packets and the Ring */
    *Delay = 0;
    return STATUS_SUCCESS;
}

static VOID FreePacketBuffers(STREAM_CONTEXT *StreamCtx)
{
    for (ULONG i = 0; i < MAX_PACKETS; i++) {
        if (StreamCtx->Packets[i] != NULL) {
            ExFreePoolWithTag(StreamCtx->Packets[i], POOL_TAG);
            StreamCtx->Packets[i] = NULL;
        }
    }
    StreamCtx->PacketCount = 0;
}

static VOID EvtStreamFreeRtPackets(ACXSTREAM Stream, PACX_RTPACKET Packets, ULONG PacketCount)
{
    for (ULONG i = 0; i < PacketCount; i++) {
        if (Packets[i].RtPacketBuffer.u.MdlType.Mdl != NULL) {
            IoFreeMdl(Packets[i].RtPacketBuffer.u.MdlType.Mdl);
        }
    }
    ExFreePoolWithTag(Packets, POOL_TAG);
    FreePacketBuffers(StreamContext(Stream));
}

/*
 * Page-aligned, zeroed packets (as in Microsoft's ACX sample): packet 0 ends on
 * a page boundary and packet 1 starts on one, so no other kernel memory is ever
 * mapped into the audio engine.
 */
static NTSTATUS EvtStreamAllocateRtPackets(ACXSTREAM Stream, ULONG PacketCount, ULONG PacketSize,
                                           PACX_RTPACKET *Packets)
{
    STREAM_CONTEXT *streamCtx = StreamContext(Stream);
    PACX_RTPACKET packets;
    ULONG allocation;
    NTSTATUS status;

    *Packets = NULL;
    if (PacketCount == 0 || PacketCount > MAX_PACKETS || PacketSize == 0 ||
        PacketSize % streamCtx->BytesPerFrame != 0) {
        return STATUS_INVALID_PARAMETER;
    }
    status = RtlULongAdd(PacketSize, PAGE_SIZE - 1, &allocation);
    if (!NT_SUCCESS(status)) {
        return status;
    }
    allocation = (allocation / PAGE_SIZE) * PAGE_SIZE;

    /*
     * CodeQL's padding check doesn't know ExAllocatePool2 zeroes the allocation
     * (no POOL_FLAG_UNINITIALIZED), padding included; and the array goes to ACX,
     * never to user mode. Microsoft's ACX sample allocates it the same way.
     */
    /* codeql[cpp/paddingbyteinformationdisclosure] */
    packets = (PACX_RTPACKET)ExAllocatePool2(POOL_FLAG_NON_PAGED,
                                             sizeof(ACX_RTPACKET) * PacketCount, POOL_TAG);
    if (packets == NULL) {
        return STATUS_INSUFFICIENT_RESOURCES;
    }

    for (ULONG i = 0; i < PacketCount; i++) {
        PMDL mdl;

        ACX_RTPACKET_INIT(&packets[i]);
        streamCtx->Packets[i] = ExAllocatePool2(POOL_FLAG_NON_PAGED, allocation, POOL_TAG);
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
        EvtStreamFreeRtPackets(Stream, packets, PacketCount);
        return status;
    }

    streamCtx->PacketCount = PacketCount;
    streamCtx->PacketSize = PacketSize;
    streamCtx->FirstPacketOffset = allocation - PacketSize;

    /* The engine's buffering: PacketCount packets of PacketFrames each. */
    TraceLoggingWrite(TraceProvider, "PacketsAllocated", TraceLoggingLevel(TRACE_LEVEL_INFO),
                      TraceLoggingUInt32(streamCtx->Device->DeviceId, "DeviceId"),
                      TraceLoggingUInt32(PacketCount, "PacketCount"),
                      TraceLoggingUInt32(PacketSize, "PacketBytes"),
                      TraceLoggingUInt32(PacketSize / streamCtx->BytesPerFrame, "PacketFrames"));
    *Packets = packets;
    return STATUS_SUCCESS;
}

static NTSTATUS EvtStreamGetCurrentPacket(ACXSTREAM Stream, PULONG CurrentPacket)
{
    *CurrentPacket = (ULONG)InterlockedCompareExchange(&StreamContext(Stream)->CurrentPacket, 0, 0);
    return STATUS_SUCCESS;
}

static NTSTATUS EvtStreamGetCapturePacket(ACXSTREAM Stream, PULONG LastCapturePacket,
                                          PULONGLONG QPCPacketStart, PBOOLEAN MoreData)
{
    STREAM_CONTEXT *streamCtx = StreamContext(Stream);

    *LastCapturePacket = (ULONG)InterlockedCompareExchange(&streamCtx->CurrentPacket, 0, 0) - 1;
    *QPCPacketStart = (ULONGLONG)InterlockedCompareExchange64(&streamCtx->LastPacketStart, 0, 0);
    *MoreData = FALSE;
    return STATUS_SUCCESS;
}

/* The engine must fill exactly the next packet: anything else is late or early. */
static NTSTATUS EvtStreamSetRenderPacket(ACXSTREAM Stream, ULONG Packet, ULONG Flags,
                                         ULONG EosPacketLength)
{
    ULONG current = (ULONG)InterlockedCompareExchange(&StreamContext(Stream)->CurrentPacket, 0, 0);

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

static NTSTATUS EvtStreamGetPresentationPosition(ACXSTREAM Stream, PULONGLONG PositionInBlocks,
                                                 PULONGLONG QPCPosition)
{
    STREAM_CONTEXT *streamCtx = StreamContext(Stream);

    *QPCPosition = Now();
    *PositionInBlocks = Position(streamCtx); /* one block is one frame, every channel */
    return STATUS_SUCCESS;
}
