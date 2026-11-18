//! Removing the root device, its endpoints and the driver packages.

use super::*;

/// Removes every root device, the leftover endpoint children and the driver
/// packages. `log` gets each step's line.
pub fn uninstall(log: &mut dyn FnMut(&str)) -> Result<Outcome, String> {
    require_admin()?;
    let mut reboots = Vec::new();
    // Roots first: removing one removes its live children. A fresh list for
    // the endpoints then finds only the leftovers (not present).
    for hardware_id in [ROOT_HARDWARE_ID, ENDPOINT_HARDWARE_ID] {
        let set = DeviceSet::all()?;
        for data in set.with_hardware_id(hardware_id) {
            let id = set.instance_id(&data);
            let mut needs_reboot = BOOL(0);
            // SAFETY: `data` belongs to `set`; no window.
            unsafe { DiUninstallDevice(HWND::default(), set.0, &data, 0, Some(&mut needs_reboot)) }
                .map_err(|e| format!("removing the device {id} failed: {e}"))?;
            reboots.push(needs_reboot.as_bool());
            log(&format!("removed the device {id}"));
        }
    }
    let packages = driver_packages()?;
    if packages.is_empty() {
        log("no Pattie driver package in the driver store");
    }
    for name in packages {
        // SAFETY: the published name (oemN.inf) of a package in the store.
        if !unsafe { SetupUninstallOEMInfW(&HSTRING::from(name.as_str()), SUOI_FORCEDELETE, None) }
            .as_bool()
        {
            return Err(format!(
                "removing the driver package {name} failed: {}",
                windows::core::Error::from_thread()
            ));
        }
        log(&format!("removed the driver package {name}"));
    }
    let outcome = uninstall_outcome(reboots);
    log(&outcome.to_string());
    Ok(outcome)
}

/// The published names (`oemN.inf`) of the driver store packages that install
/// `Root\Pattie`.
fn driver_packages() -> Result<Vec<String>, String> {
    let windows_dir =
        std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    let inf_dir = windows_dir.join("INF");
    let entries = std::fs::read_dir(&inf_dir)
        .map_err(|e| format!("can't list {}: {e}", inf_dir.display()))?;
    let mut names = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let lower = name.to_ascii_lowercase();
        if !(lower.starts_with("oem") && lower.ends_with(".inf")) {
            continue;
        }
        if let Ok(bytes) = std::fs::read(entry.path())
            && inf_lists_hardware_id(&decode_inf(&bytes), ROOT_HARDWARE_ID)
        {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}
