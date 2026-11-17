//! The capture loop: clicks heard on a recording stream.

use super::*;

pub(in crate::commands::measure) fn capture_clicks(
    endpoint: SendDevice,
    stop: &AtomicBool,
    run: &Mutex<Run>,
) -> Result<(), String> {
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
