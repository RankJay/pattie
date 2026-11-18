//! measure cable: clicks from one Windows endpoint to another, through a loopback.

use super::*;

/// `pattie-cli measure cable`: another virtual device as a baseline (VB-CABLE's
/// "CABLE Input" and "CABLE Output", say). Clicks are rendered into `render`, as
/// an app feeding the cable does, and found in `capture`, as a meeting app
/// recording it does: the whole app-to-app path, the counterpart of both
/// `measure mic` and `measure speaker`, measured the same way.
pub fn cable(render: &str, capture: &str, seconds: u64) -> Result<(), String> {
    // SAFETY: initializes COM for this thread; the threads we spawn do their own.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .map_err(|e| format!("COM: {e}"))?;
    let render = SendDevice(endpoint_named(eRender, render)?);
    let capture = SendDevice(endpoint_named(eCapture, capture)?);
    let stop = Arc::new(AtomicBool::new(false));
    let run = Arc::new(Mutex::new(Run::default()));
    let capturing = {
        let (stop, run) = (stop.clone(), run.clone());
        thread::spawn(move || capture_clicks(capture, &stop, &run))
    };
    let rendering = {
        let (stop, run) = (stop.clone(), run.clone());
        thread::spawn(move || render_clicks(render, &stop, &run))
    };
    println!("measuring for {seconds} s (both ends event-driven WASAPI) ...");
    #[expect(
        clippy::disallowed_methods,
        reason = "the measuring window; both streams run on their own threads"
    )]
    thread::sleep(Duration::from_secs(seconds));
    stop.store(true, Ordering::Relaxed);
    for (side, handle) in [("render", rendering), ("capture", capturing)] {
        handle
            .join()
            .map_err(|_| format!("{side} thread panicked"))??;
    }
    let run = std::mem::take(&mut *run.lock().unwrap());
    report("cable", &run, None);
    Ok(())
}

/// The active endpoint whose friendly name contains `wanted` (`pick_endpoint`).
fn endpoint_named(flow: EDataFlow, wanted: &str) -> Result<IMMDevice, String> {
    // SAFETY: standard COM calls on objects this function owns.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|e| format!("device enumerator: {e}"))?;
        let devices = enumerator
            .EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)
            .map_err(|e| format!("enumerate endpoints: {e}"))?;
        let mut found = Vec::new();
        for i in 0..devices.GetCount().unwrap_or(0) {
            let Ok(device) = devices.Item(i) else {
                continue;
            };
            let Ok(store) = device.OpenPropertyStore(STGM_READ) else {
                continue;
            };
            let Ok(value) = store.GetValue(&PKEY_Device_FriendlyName) else {
                continue;
            };
            let Ok(text) = PropVariantToStringAlloc(&value) else {
                continue;
            };
            let name = text.to_string().unwrap_or_default();
            CoTaskMemFree(Some(text.0 as _));
            found.push((name, device));
        }
        let names: Vec<String> = found.iter().map(|(name, _)| name.clone()).collect();
        let index = pick_endpoint(&names, wanted)?;
        let (name, device) = found.swap_remove(index);
        println!("endpoint \"{name}\"");
        Ok(device)
    }
}
