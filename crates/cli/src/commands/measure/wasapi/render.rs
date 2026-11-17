//! The render loop: clicks played on a render stream.

use super::*;

pub(in crate::commands::measure) fn render_clicks(
    endpoint: SendDevice,
    stop: &AtomicBool,
    run: &Mutex<Run>,
) -> Result<(), String> {
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
            let click_frames = (stream.rate / CLICKS_PER_SECOND) as usize;
            for i in 0..samples {
                let value = if click && i / stream.channels < click_frames {
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
