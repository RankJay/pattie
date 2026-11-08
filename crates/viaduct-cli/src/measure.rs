//! `viaduct-cli measure mic|speaker <name> [--seconds N]`: latency and glitch
//! measurement, with WASAPI playing the part a real app (Zoom) would.
//!
//! Every 500 ms a 1 ms click goes in one side and is timed coming out the other,
//! with both ends on the performance counter:
//!
//! - mic: the CLI writes clicks into the Device; WASAPI captures the endpoint in
//!   shared, event-driven mode and stamps each buffer with its capture time.
//! - speaker: WASAPI renders clicks to the endpoint; the SDK reader catches them.
//!
//! Raw latency is click written to click seen. Added latency subtracts the time the
//! click spent queued behind audio the app itself had already written (its lead):
//! that's the brief's "added latency, app-to-device", checked against 10 ms.

// Scoped exception: the WASAPI blocks below are each one conversation with
// COM objects this module owns (create, activate, initialize, loop, release).
// Splitting every COM call into its own block would repeat one safety argument
// dozens of times. Pointer casts and buffer access stay separate and checked.
#![allow(
    clippy::multiple_unsafe_ops_per_block,
    reason = "each block is one WASAPI/COM call sequence with a single safety argument"
)]

use crate::latency::{ClickDetector, HNS_PER_SECOND, Sent, frames_to_hns, pair_clicks, spread};
use crate::pace::Pacer;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use viaduct::{Device, Kind};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, DEVICE_STATE_ACTIVE, EDataFlow, IAudioCaptureClient,
    IAudioClient, IAudioRenderClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    WAVEFORMATEX, eCapture, eRender,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, STGM_READ,
};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::core::GUID;

const RING_RATE: u64 = 48_000;
const CLICK_PERIOD: Duration = Duration::from_millis(500);
const CLICK_FRAMES: usize = 48; // 1 ms at 48 kHz
const CLICK_LEVEL: f32 = 0.8;

/// An audio endpoint's device instance path (PKEY_Device_InstanceId's endpoint
/// counterpart, {b3f8fa53-0004-438e-9003-51a46e139bfc},2): for a Viaduct endpoint,
/// `...VIADUCT\ENDPOINT\...&<Device ID>`.
const PKEY_ENDPOINT_DEVICE_INSTANCE: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xb3f8fa53_0004_438e_9003_51a46e139bfc),
    pid: 2,
};

/// Now, in 100 ns units on the performance counter (WASAPI's QPC timestamps use the same).
fn now_hns() -> u64 {
    let (mut counter, mut frequency) = (0i64, 0i64);
    // SAFETY: a plain out-parameter.
    let _ = unsafe { QueryPerformanceCounter(&mut counter) };
    // SAFETY: a plain out-parameter.
    let _ = unsafe { QueryPerformanceFrequency(&mut frequency) };
    (counter as u128 * HNS_PER_SECOND as u128 / frequency as u128) as u64
}

/// What a run collected.
#[derive(Default)]
struct Run {
    sent: Vec<Sent>,
    /// When each click was stamped: the mic's capture timestamp from WASAPI.
    seen_hns: Vec<u64>,
    /// When each click actually reached the app (same index as `seen_hns`). For
    /// the mic this is later than the stamp: a packet is stamped with its start
    /// but handed over when it completes, exactly as hardware does.
    delivered_hns: Vec<u64>,
    discontinuities: u64,
    /// The CLI's own loop (mic writer, speaker reader): how it was paced, its
    /// longest gap between passes, and how many gaps exceeded `gap_limit`. A mic
    /// Gap with no writer gap over the lead points at the driver, not the app.
    pacing: &'static str,
    longest_gap: Duration,
    gaps_over: u64,
    gap_limit: Duration,
}

pub fn run(direction: &str, name: &str, seconds: u64, lead_ms: u64) -> Result<(), String> {
    // SAFETY: initializes COM for this thread; the threads we spawn do their own.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .map_err(|e| format!("COM: {e}"))?;
    let duration = Duration::from_secs(seconds);
    let (run, stats_before, stats_after) = match direction {
        "mic" => measure_mic(name, duration, lead_ms)?,
        "speaker" => measure_speaker(name, duration)?,
        _ => return Err("measure needs `mic` or `speaker`".into()),
    };
    report(direction, &run, stats_before, stats_after);
    Ok(())
}

/// Finds the active endpoint for a Device, waiting while Windows sets it up (the
/// first one after a driver install takes several seconds).
///
/// Matched by the endpoint's device instance path, which ends in the circuit
/// device's stable instance ID (`&M<hex>` / `&S<hex>`). The name is a fallback.
fn find_endpoint(flow: EDataFlow, name: &str, kind: Kind) -> Result<IMMDevice, String> {
    let instance_suffix = format!("&{}", viaduct::stable_id(kind, name));
    let name_suffixes = [format!("({name})"), format!("- {name})")];
    let deadline = Instant::now() + Duration::from_secs(30);
    let started = Instant::now();
    loop {
        // SAFETY: standard COM calls on objects this function owns.
        let found = unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .map_err(|e| format!("device enumerator: {e}"))?;
            let devices = enumerator
                .EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)
                .map_err(|e| format!("enumerate endpoints: {e}"))?;
            let mut found = None;
            for i in 0..devices.GetCount().unwrap_or(0) {
                let Ok(device) = devices.Item(i) else {
                    continue;
                };
                let Ok(store) = device.OpenPropertyStore(STGM_READ) else {
                    continue;
                };
                let text_of = |key: &PROPERTYKEY| -> String {
                    let Ok(value) = store.GetValue(key) else {
                        return String::new();
                    };
                    let Ok(text) = PropVariantToStringAlloc(&value) else {
                        return String::new();
                    };
                    let string = text.to_string().unwrap_or_default();
                    CoTaskMemFree(Some(text.0 as _));
                    string
                };
                let friendly = text_of(&PKEY_Device_FriendlyName);
                let instance = text_of(&PKEY_ENDPOINT_DEVICE_INSTANCE).to_uppercase();
                let by_instance =
                    instance.contains(r"VIADUCT\ENDPOINT\") && instance.ends_with(&instance_suffix);
                let by_name = name_suffixes.iter().any(|s| friendly.ends_with(s.as_str()));
                if by_instance || (instance.is_empty() && by_name) {
                    println!(
                        "endpoint \"{friendly}\" ({}) found after {:.1} s",
                        if by_instance {
                            instance.as_str()
                        } else {
                            "matched by name"
                        },
                        started.elapsed().as_secs_f32()
                    );
                    found = Some(device);
                    break;
                }
            }
            found
        };
        if let Some(device) = found {
            return Ok(device);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "no active endpoint {instance_suffix} (\"{name}\") after 30 s"
            ));
        }
        #[expect(
            clippy::disallowed_methods,
            reason = "a coarse poll while Windows sets the endpoint up; no audio flows yet"
        )]
        thread::sleep(Duration::from_millis(250));
    }
}

/// An event-driven, shared-mode audio client on `device`, as a typical app opens it.
struct Stream {
    client: IAudioClient,
    event: HANDLE,
    channels: usize,
    rate: u64,
    float: bool,
}

impl Stream {
    fn open(device: &IMMDevice) -> Result<Stream, String> {
        // SAFETY: standard WASAPI setup; the mix format is freed after use.
        unsafe {
            let client: IAudioClient = device
                .Activate(CLSCTX_ALL, None)
                .map_err(|e| format!("activate audio client: {e}"))?;
            let format: *mut WAVEFORMATEX = client
                .GetMixFormat()
                .map_err(|e| format!("mix format: {e}"))?;
            let (channels, rate, bits) = (
                (*format).nChannels as usize,
                (*format).nSamplesPerSec as u64,
                (*format).wBitsPerSample,
            );
            let initialized = client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                0,
                0,
                format,
                None,
            );
            CoTaskMemFree(Some(format as _));
            initialized.map_err(|e| format!("initialize audio client: {e}"))?;
            let event =
                CreateEventW(None, false, false, None).map_err(|e| format!("event: {e}"))?;
            client
                .SetEventHandle(event)
                .map_err(|e| format!("set event: {e}"))?;
            println!(
                "WASAPI shared mode: {rate} Hz, {channels} channel(s), {bits}-bit {}",
                if bits == 32 { "float" } else { "PCM" }
            );
            Ok(Stream {
                client,
                event,
                channels,
                rate,
                float: bits == 32,
            })
        }
    }

    /// Channel 0 of interleaved frame `frame` in a WASAPI buffer.
    fn sample(&self, data: *const u8, frame: usize) -> f32 {
        if self.float {
            // SAFETY: callers pass a frame inside the buffer WASAPI handed out.
            let at = unsafe { data.add(frame * self.channels * 4) };
            // SAFETY: four readable bytes of that buffer; read unaligned, so no
            // alignment is assumed of WASAPI's pointer.
            unsafe { at.cast::<f32>().read_unaligned() }
        } else {
            // SAFETY: as above, two bytes per sample.
            let at = unsafe { data.add(frame * self.channels * 2) };
            // SAFETY: as above.
            f32::from(unsafe { at.cast::<i16>().read_unaligned() }) / 32768.0
        }
    }

    fn wait(&self) -> bool {
        // SAFETY: the event is open until Drop.
        unsafe { WaitForSingleObject(self.event, 200) == WAIT_OBJECT_0 }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        // SAFETY: stopping a started or unstarted client is harmless; the event is ours.
        unsafe {
            let _ = self.client.Stop();
            let _ = CloseHandle(self.event);
        }
    }
}

fn measure_mic(
    name: &str,
    duration: Duration,
    lead_ms: u64,
) -> Result<(Run, viaduct::Stats, viaduct::Stats), String> {
    let mic = Device::builder()
        .name(name)
        .kind(Kind::Microphone)
        .format(48_000, 1)
        .create()
        .map_err(|e| format!("couldn't create the microphone: {e}"))?;
    let mut tx = mic.writer().map_err(|e| e.to_string())?;
    let endpoint = find_endpoint(eCapture, name, Kind::Microphone)?;
    let stop = Arc::new(AtomicBool::new(false));
    let run = Arc::new(Mutex::new(Run::default()));

    let capture = {
        let (stop, run) = (stop.clone(), run.clone());
        // IMMDevice is free-threaded (COM MTA); move it to the capture thread.
        let endpoint = SendDevice(endpoint);
        thread::spawn(move || capture_clicks(endpoint, &stop, &run))
    };

    let stats_before = mic.stats();
    let start = Instant::now();
    // Paced by fill level, not by time: top the Ring up to LEAD each millisecond.
    // Time-based pacing writes while Windows isn't reading yet, and that backlog
    // then never drains (both sides run at 48 kHz), adding its length to every click.
    let lead = RING_RATE * lead_ms / 1000;
    let mut pacer = Pacer::new(Duration::from_millis(lead_ms));
    let mut next_click = Duration::from_secs(1); // let the capture side settle first
    let mut chunk = Vec::new();
    println!(
        "measuring for {} s, keeping a {lead_ms} ms lead ({}) ...",
        duration.as_secs(),
        pacer.describe()
    );
    while start.elapsed() < duration {
        pacer.tick();
        let ahead = mic.stats().buffered_frames;
        if ahead < lead {
            chunk.clear();
            chunk.resize((lead - ahead) as usize, 0.0f32);
            if start.elapsed() >= next_click && chunk.len() >= CLICK_FRAMES {
                chunk[..CLICK_FRAMES].fill(CLICK_LEVEL);
                run.lock().unwrap().sent.push(Sent {
                    at_hns: now_hns(),
                    ahead_hns: frames_to_hns(ahead, RING_RATE),
                });
                next_click += CLICK_PERIOD;
            }
            tx.write(&chunk).map_err(|e| e.to_string())?;
        }
        pacer.sleep_1ms();
    }
    // Read before stopping: once the writer stops, the driver plays out the lead
    // and then finds the Ring empty while capture winds down. That's one packet of
    // Gap after the measuring window, not during it.
    let stats_after = mic.stats();
    stop.store(true, Ordering::Relaxed);
    capture
        .join()
        .map_err(|_| "capture thread panicked".to_string())??;
    let mut run = std::mem::take(&mut *run.lock().unwrap());
    (run.longest_gap, run.gaps_over) = pacer.gaps();
    (run.pacing, run.gap_limit) = (pacer.describe(), Duration::from_millis(lead_ms));
    Ok((run, stats_before, stats_after))
}

struct SendDevice(IMMDevice);
// SAFETY: MMDevice objects are free-threaded and this program runs COM in the MTA.
unsafe impl Send for SendDevice {}

fn capture_clicks(endpoint: SendDevice, stop: &AtomicBool, run: &Mutex<Run>) -> Result<(), String> {
    // SAFETY: joins this thread to the process's MTA.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .map_err(|e| e.to_string())?;
    let stream = Stream::open(&endpoint.0)?;
    // SAFETY: standard WASAPI capture loop; each buffer is released after reading.
    unsafe {
        let capture: IAudioCaptureClient = stream
            .client
            .GetService()
            .map_err(|e| format!("capture client: {e}"))?;
        stream
            .client
            .Start()
            .map_err(|e| format!("start capture: {e}"))?;
        let mut detector = ClickDetector::default();
        while !stop.load(Ordering::Relaxed) {
            if !stream.wait() {
                continue;
            }
            while capture.GetNextPacketSize().unwrap_or(0) > 0 {
                let (mut data, mut frames, mut flags, mut qpc) =
                    (std::ptr::null_mut(), 0u32, 0u32, 0u64);
                if capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut qpc))
                    .is_err()
                {
                    break;
                }
                let delivered = now_hns();
                let mut run = run.lock().unwrap();
                if flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0 {
                    run.discontinuities += 1;
                }
                for frame in 0..frames as usize {
                    let at = qpc + frames_to_hns(frame as u64, stream.rate);
                    if detector.detect(at, stream.sample(data, frame)) {
                        run.seen_hns.push(at);
                        run.delivered_hns.push(delivered);
                    }
                }
                drop(run);
                let _ = capture.ReleaseBuffer(frames);
            }
        }
    }
    Ok(())
}

fn measure_speaker(
    name: &str,
    duration: Duration,
) -> Result<(Run, viaduct::Stats, viaduct::Stats), String> {
    let speaker = Device::builder()
        .name(name)
        .kind(Kind::Speaker)
        .format(48_000, 1)
        .create()
        .map_err(|e| format!("couldn't create the speaker: {e}"))?;
    let mut rx = speaker.reader().map_err(|e| e.to_string())?;
    let endpoint = find_endpoint(eRender, name, Kind::Speaker)?;
    let stop = Arc::new(AtomicBool::new(false));
    let run = Arc::new(Mutex::new(Run::default()));

    let render = {
        let (stop, run) = (stop.clone(), run.clone());
        let endpoint = SendDevice(endpoint);
        thread::spawn(move || render_clicks(endpoint, &stop, &run))
    };

    let stats_before = speaker.stats();
    let start = Instant::now();
    let mut buffer = vec![0.0f32; 4800];
    let mut detector = ClickDetector::default();
    // Arrival is known only to within the polling gap: report gaps over 2 ms.
    let mut pacer = Pacer::new(Duration::from_millis(2));
    println!(
        "measuring for {} s ({}) ...",
        duration.as_secs(),
        pacer.describe()
    );
    while start.elapsed() < duration {
        pacer.tick();
        let count = rx.read(&mut buffer).map_err(|e| e.to_string())?;
        let read_at = now_hns();
        // Polled every millisecond: arrival is known to within ~1 ms.
        let loudest = buffer[..count].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        if detector.detect(read_at, loudest) {
            let mut run = run.lock().unwrap();
            run.seen_hns.push(read_at);
            run.delivered_hns.push(read_at); // the reader is the app: seen is delivered
        }
        pacer.sleep_1ms();
    }
    // Read before stopping, as for the mic: once the reader stops, the driver keeps
    // writing until playback stops, which is after the measuring window.
    let stats_after = speaker.stats();
    stop.store(true, Ordering::Relaxed);
    render
        .join()
        .map_err(|_| "render thread panicked".to_string())??;
    let mut run = std::mem::take(&mut *run.lock().unwrap());
    (run.longest_gap, run.gaps_over) = pacer.gaps();
    (run.pacing, run.gap_limit) = (pacer.describe(), Duration::from_millis(2));
    Ok((run, stats_before, stats_after))
}

fn render_clicks(endpoint: SendDevice, stop: &AtomicBool, run: &Mutex<Run>) -> Result<(), String> {
    // SAFETY: joins this thread to the process's MTA.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .map_err(|e| e.to_string())?;
    let stream = Stream::open(&endpoint.0)?;
    // SAFETY: standard WASAPI render loop; each buffer is released after filling.
    unsafe {
        let render: IAudioRenderClient = stream
            .client
            .GetService()
            .map_err(|e| format!("render client: {e}"))?;
        let size = stream
            .client
            .GetBufferSize()
            .map_err(|e| format!("buffer size: {e}"))?;
        stream
            .client
            .Start()
            .map_err(|e| format!("start render: {e}"))?;
        let start = Instant::now();
        let mut next_click = Duration::from_secs(1);
        while !stop.load(Ordering::Relaxed) {
            if !stream.wait() {
                continue;
            }
            let padding = stream.client.GetCurrentPadding().unwrap_or(size);
            let frames = size.saturating_sub(padding);
            if frames == 0 {
                continue;
            }
            let Ok(data) = render.GetBuffer(frames) else {
                continue;
            };
            let samples = frames as usize * stream.channels;
            let click = start.elapsed() >= next_click;
            for i in 0..samples {
                let value = if click && i / stream.channels < CLICK_FRAMES {
                    CLICK_LEVEL
                } else {
                    0.0
                };
                if stream.float {
                    data.add(i * 4).cast::<f32>().write_unaligned(value);
                } else {
                    data.add(i * 2)
                        .cast::<i16>()
                        .write_unaligned((value * 32767.0) as i16);
                }
            }
            if click {
                run.lock().unwrap().sent.push(Sent {
                    at_hns: now_hns(),
                    ahead_hns: frames_to_hns(padding as u64, stream.rate),
                });
                next_click += CLICK_PERIOD;
            }
            let _ = render.ReleaseBuffer(frames, 0);
        }
    }
    Ok(())
}

/// Pairs each click seen with the latest one sent before it, then prints the spread.
fn report(direction: &str, run: &Run, before: viaduct::Stats, after: viaduct::Stats) {
    let mut latencies = pair_clicks(&run.sent, &run.seen_hns, &run.delivered_hns);
    let (raw, added) = (&mut latencies.raw, &mut latencies.added);
    let (raw_delivered, added_delivered) =
        (&mut latencies.raw_delivered, &mut latencies.added_delivered);

    println!();
    println!(
        "== {direction}: {} clicks sent, {} matched",
        run.sent.len(),
        raw.len()
    );
    if direction == "mic" {
        println!("by capture timestamp (how apps sync audio):");
        println!("  raw   (ms) {}", spread(raw));
        println!("  added (ms) {}", spread(added));
        println!("by delivery to the app (what a live call experiences):");
    }
    println!("  raw   (ms) {}", spread(raw_delivered));
    println!(
        "  added (ms) {}   <- brief's target: under 10 ms",
        spread(added_delivered)
    );
    println!(
        "glitches: {} WASAPI discontinuities, {} Gap frames, {} Overflow frames during the run",
        run.discontinuities,
        after.gap_frames - before.gap_frames,
        after.overflow_frames - before.overflow_frames
    );
    println!(
        "{} loop ({}): longest gap {:.1} ms, {} gaps over {} ms",
        if direction == "mic" {
            "writer"
        } else {
            "reader"
        },
        run.pacing,
        run.longest_gap.as_secs_f64() * 1000.0,
        run.gaps_over,
        run.gap_limit.as_millis()
    );
}
