//! `pattie-cli measure mic|speaker <name> [--seconds N]`: latency and glitch
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
//! that's the brief's "added latency, app-to-device": the gate (below VB-CABLE,
//! under 45 ms mic / 32 ms speaker) and the original goal, under 10 ms.

// Scoped exception: the WASAPI blocks below are each one conversation with
// COM objects this module owns (create, activate, initialize, loop, release).
// Splitting every COM call into its own block would repeat one safety argument
// dozens of times. Pointer casts and buffer access stay separate and checked.
#![allow(
    clippy::multiple_unsafe_ops_per_block,
    reason = "each block is one WASAPI/COM call sequence with a single safety argument"
)]

use crate::args::DeviceFormat;
use crate::clicks::{
    ClickDetector, HNS_PER_SECOND, Sent, frames_to_hns, pair_clicks, spread, target_note,
};
use crate::gap_meter::GapMeter;
use crate::pick_endpoint::pick_endpoint;
use pattie::{Device, Kind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
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

mod cable;
mod drift;
mod endpoints;
mod mic;
mod speaker;
mod wasapi;

pub use cable::cable;
pub use drift::drift;
use endpoints::find_endpoint;
use mic::measure_mic;
use speaker::measure_speaker;
use wasapi::{capture_clicks, render_clicks};

const CLICK_PERIOD: Duration = Duration::from_millis(500);
/// A click lasts 1 ms: a thousandth of a second of frames.
const CLICKS_PER_SECOND: u64 = 1_000;
const CLICK_LEVEL: f32 = 0.8;

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

pub fn run(direction: &str, name: &str, seconds: u64, format: DeviceFormat) -> Result<(), String> {
    // SAFETY: initializes COM for this thread; the threads we spawn do their own.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .map_err(|e| format!("COM: {e}"))?;
    let duration = Duration::from_secs(seconds);
    let (run, stats_before, stats_after) = match direction {
        "mic" => measure_mic(name, duration, format)?,
        "speaker" => measure_speaker(name, duration, format)?,
        _ => return Err("measure needs `mic` or `speaker`".into()),
    };
    report(direction, &run, Some((stats_before, stats_after)));
    Ok(())
}

struct SendDevice(IMMDevice);
// SAFETY: MMDevice objects are free-threaded and this program runs COM in the MTA.
unsafe impl Send for SendDevice {}

/// Pairs each click seen with the latest one sent before it, then prints the spread.
/// Prints the spread. `stats` is the Device's (before, after) for `mic` and
/// `speaker`; `None` for `cable`, which has no Pattie Device.
fn report(direction: &str, run: &Run, stats: Option<(pattie::Stats, pattie::Stats)>) {
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
    if direction == "mic" || direction == "cable" {
        println!("by capture timestamp (how apps sync audio):");
    } else {
        println!("by the device's playback timestamp (Reader::read_timed):");
    }
    println!("  raw   (ms) {}", spread(raw));
    println!("  added (ms) {}", spread(added));
    println!("by delivery to the app (what a live call experiences):");
    println!("  raw   (ms) {}", spread(raw_delivered));
    println!(
        "  added (ms) {}   <- {}",
        spread(added_delivered),
        target_note(direction)
    );
    let Some((before, after)) = stats else {
        println!(
            "glitches: {} WASAPI discontinuities during the run",
            run.discontinuities
        );
        return;
    };
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
