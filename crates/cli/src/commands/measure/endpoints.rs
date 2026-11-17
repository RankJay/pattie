//! Finding the Windows endpoint of a Device.

use super::*;

/// An audio endpoint's device instance path (PKEY_Device_InstanceId's endpoint
/// counterpart, {b3f8fa53-0004-438e-9003-51a46e139bfc},2): for a Pattie endpoint,
/// `...PATTIE\ENDPOINT\...&<Device ID>`.
const PKEY_ENDPOINT_DEVICE_INSTANCE: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xb3f8fa53_0004_438e_9003_51a46e139bfc),
    pid: 2,
};

/// Finds the active endpoint for a Device, waiting while Windows sets it up (the
/// first one after a driver install takes several seconds).
///
/// Matched by the endpoint's device instance path, which ends in the circuit
/// device's stable instance ID (`&M<hex>` / `&S<hex>`, from kind, name and
/// format). The name is a fallback.
pub(super) fn find_endpoint(
    flow: EDataFlow,
    name: &str,
    kind: Kind,
    format: DeviceFormat,
) -> Result<IMMDevice, String> {
    let instance_suffix = format!(
        "&{}",
        pattie::stable_id(kind, name, format.rate, format.channels)
    );
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
                    instance.contains(r"PATTIE\ENDPOINT\") && instance.ends_with(&instance_suffix);
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
