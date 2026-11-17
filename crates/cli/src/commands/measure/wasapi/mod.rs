//! Stream: one WASAPI client, and the two loops that drive one.

use super::*;

mod capture;
mod render;

pub(super) use capture::capture_clicks;
pub(super) use render::render_clicks;

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
