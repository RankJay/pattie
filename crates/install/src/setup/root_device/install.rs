//! Installing the driver package on the root device.

use super::*;

/// The package's INF as an absolute path: `package` is the INF or its folder.
fn package_inf(package: &str) -> Result<PathBuf, String> {
    let path = Path::new(package);
    let inf = if path.is_dir() {
        path.join("pattie.inf")
    } else {
        path.to_path_buf()
    };
    let inf = std::path::absolute(&inf).map_err(|e| format!("{}: {e}", inf.display()))?;
    let bytes = std::fs::read(&inf).map_err(|e| format!("can't read {}: {e}", inf.display()))?;
    if !inf_lists_hardware_id(&decode_inf(&bytes), ROOT_HARDWARE_ID) {
        return Err(format!(
            "{} is not the Pattie driver's INF (no {ROOT_HARDWARE_ID})",
            inf.display()
        ));
    }
    Ok(inf)
}

/// Installs the package (its folder or `pattie.inf`) on the root device,
/// creating that device if there is none. `log` gets each step's line.
pub fn install(package: &str, log: &mut dyn FnMut(&str)) -> Result<Outcome, String> {
    require_admin()?;
    let inf = package_inf(package)?;

    let existing = {
        let set = DeviceSet::all()?;
        let found = set.with_hardware_id(ROOT_HARDWARE_ID);
        for data in &found {
            log(&format!(
                "root device already present: {}",
                set.instance_id(data)
            ));
        }
        found.len()
    };
    if existing > 1 {
        log(&format!(
            "warning: {existing} root devices; run `pattie-cli uninstall`, then install again, to keep one"
        ));
    }
    let created = if existing == 0 {
        Some(NewRootDevice::create(&inf, log)?)
    } else {
        None
    };

    let mut reboot = BOOL(0);
    // Non-interactive, as devcon's install: a driver Windows won't trust fails
    // here instead of waiting on a dialog.
    // SAFETY: a process-wide setup flag, restored right after.
    let was_non_interactive = unsafe { SetupSetNonInteractiveMode(true) };
    // SAFETY: both strings are NUL-terminated HSTRINGs alive for the call.
    let result = unsafe {
        UpdateDriverForPlugAndPlayDevicesW(
            None,
            &HSTRING::from(ROOT_HARDWARE_ID),
            &HSTRING::from(inf.as_path()),
            UPDATEDRIVERFORPLUGANDPLAYDEVICES_FLAGS(0),
            Some(&mut reboot),
        )
    };
    // SAFETY: as above.
    unsafe {
        let _ = SetupSetNonInteractiveMode(was_non_interactive.as_bool());
    }
    let update = match result {
        Ok(()) => DriverUpdate::Installed,
        // Not forced: Windows kept the installed driver. Kept but older than this
        // package means Windows refused the package (classify_kept).
        Err(e) if e.code() == HRESULT::from_win32(ERROR_NO_MORE_ITEMS.0) => {
            let offered = std::fs::read(&inf)
                .ok()
                .and_then(|bytes| inf_driver_version(&decode_inf(&bytes)));
            classify_kept(root_driver_version()?, offered)
        }
        Err(e) => DriverUpdate::Failed(format!("{} ({e})", inf.display())),
    };
    // The flag alone misses an update that left the root waiting (ADR 0024).
    let reboot = restart_pending(reboot.as_bool(), &root_problems()?);
    match install_outcome(update, reboot, created.is_some()) {
        Ok(outcome) => {
            log(&format!("{outcome} ({})", inf.display()));
            Ok(outcome)
        }
        Err(message) => {
            if let Some(device) = created {
                device.remove(log);
            }
            Err(message)
        }
    }
}
