//! The SetupAPI half of `viaduct-cli install` / `uninstall` (ADR 0016): the calls
//! that change the machine. Checked in the test VM (VM-STEPS D.5), never run on
//! the host, and left out of coverage for that reason (README, Coverage).

use super::{
    ENDPOINT_HARDWARE_ID, ROOT_HARDWARE_ID, decode_inf, has_hardware_id, inf_lists_hardware_id,
    parse_multi_sz, to_multi_sz,
};
use std::path::{Path, PathBuf};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    DICD_GENERATE_ID, DIF_REGISTERDEVICE, DIF_REMOVE, DIGCF_ALLCLASSES, DiUninstallDevice,
    HDEVINFO, SP_DEVINFO_DATA, SPDRP_HARDWAREID, SUOI_FORCEDELETE, SetupDiCallClassInstaller,
    SetupDiCreateDeviceInfoList, SetupDiCreateDeviceInfoW, SetupDiDestroyDeviceInfoList,
    SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW,
    SetupDiGetDeviceRegistryPropertyW, SetupDiGetINFClassW, SetupDiSetDeviceRegistryPropertyW,
    SetupSetNonInteractiveMode, SetupUninstallOEMInfW, UPDATEDRIVERFORPLUGANDPLAYDEVICES_FLAGS,
    UpdateDriverForPlugAndPlayDevicesW,
};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_INSUFFICIENT_BUFFER, ERROR_NO_MORE_ITEMS, HANDLE, HWND,
};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::{BOOL, GUID, HRESULT, HSTRING, PCWSTR};

/// MAX_CLASS_NAME_LEN in setupapi.h.
const MAX_CLASS_NAME_LEN: usize = 32;

/// A device information set, destroyed on drop.
struct DeviceSet(HDEVINFO);

impl Drop for DeviceSet {
    fn drop(&mut self) {
        // SAFETY: the set came from SetupDiGetClassDevsW or
        // SetupDiCreateDeviceInfoList and is destroyed only here.
        unsafe {
            let _ = SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

impl DeviceSet {
    /// Every device on the machine, present or not.
    fn all() -> Result<Self, String> {
        // SAFETY: no class GUID, no enumerator, no window: all devices.
        unsafe { SetupDiGetClassDevsW(None, PCWSTR::null(), None, DIGCF_ALLCLASSES) }
            .map(DeviceSet)
            .map_err(|e| format!("listing devices failed: {e}"))
    }

    /// The devices in the set whose hardware IDs include `hardware_id`.
    fn with_hardware_id(&self, hardware_id: &str) -> Vec<SP_DEVINFO_DATA> {
        let mut found = Vec::new();
        for index in 0.. {
            let mut data = new_devinfo_data();
            // SAFETY: `data` has its cbSize set; the call fails past the end.
            if unsafe { SetupDiEnumDeviceInfo(self.0, index, &mut data) }.is_err() {
                break;
            }
            if has_hardware_id(&self.hardware_ids(&data), hardware_id) {
                found.push(data);
            }
        }
        found
    }

    fn hardware_ids(&self, data: &SP_DEVINFO_DATA) -> Vec<String> {
        let mut buffer = vec![0u8; 512];
        loop {
            let mut needed = 0u32;
            // SAFETY: the buffer and its length are passed together.
            let result = unsafe {
                SetupDiGetDeviceRegistryPropertyW(
                    self.0,
                    data,
                    SPDRP_HARDWAREID,
                    None,
                    Some(&mut buffer),
                    Some(&mut needed),
                )
            };
            match result {
                Ok(()) => {
                    return parse_multi_sz(&buffer[..(needed as usize).min(buffer.len())]);
                }
                Err(e)
                    if e.code() == HRESULT::from_win32(ERROR_INSUFFICIENT_BUFFER.0)
                        && needed as usize > buffer.len() =>
                {
                    buffer.resize(needed as usize, 0)
                }
                // Many devices have no hardware IDs at all.
                Err(_) => return Vec::new(),
            }
        }
    }

    fn instance_id(&self, data: &SP_DEVINFO_DATA) -> String {
        let mut buffer = [0u16; 512];
        let mut needed = 0u32;
        // SAFETY: the buffer and its length are passed together.
        match unsafe {
            SetupDiGetDeviceInstanceIdW(self.0, data, Some(&mut buffer), Some(&mut needed))
        } {
            Ok(()) => String::from_utf16_lossy(&buffer[..(needed as usize).saturating_sub(1)]),
            Err(_) => "(unknown instance)".to_string(),
        }
    }
}

fn new_devinfo_data() -> SP_DEVINFO_DATA {
    SP_DEVINFO_DATA {
        cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
        ..Default::default()
    }
}

fn require_admin() -> Result<(), String> {
    let mut token = HANDLE::default();
    // SAFETY: returns this process's pseudo-handle; nothing to free.
    let process = unsafe { GetCurrentProcess() };
    // SAFETY: a valid process handle and out-parameter; the token is closed below.
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }
        .map_err(|e| format!("checking for administrator rights failed: {e}"))?;
    let mut elevation = TOKEN_ELEVATION::default();
    let mut size = 0u32;
    // SAFETY: `elevation` is the TOKEN_ELEVATION the class asks for, with its size.
    let result = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some(std::ptr::from_mut(&mut elevation).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        )
    };
    // SAFETY: opened above, closed once.
    unsafe {
        let _ = CloseHandle(token);
    }
    result.map_err(|e| format!("checking for administrator rights failed: {e}"))?;
    if elevation.TokenIsElevated == 0 {
        return Err(
            "this needs administrator rights: run it from an administrator PowerShell".to_string(),
        );
    }
    Ok(())
}

/// The package's INF as an absolute path: `package` is the INF or its folder.
fn package_inf(package: &str) -> Result<PathBuf, String> {
    let path = Path::new(package);
    let inf = if path.is_dir() {
        path.join("viaduct.inf")
    } else {
        path.to_path_buf()
    };
    let inf = std::path::absolute(&inf).map_err(|e| format!("{}: {e}", inf.display()))?;
    let bytes = std::fs::read(&inf).map_err(|e| format!("can't read {}: {e}", inf.display()))?;
    if !inf_lists_hardware_id(&decode_inf(&bytes), ROOT_HARDWARE_ID) {
        return Err(format!(
            "{} is not the Viaduct driver's INF (no {ROOT_HARDWARE_ID})",
            inf.display()
        ));
    }
    Ok(inf)
}

/// A root device this run registered, removed again if the install fails.
struct NewRootDevice {
    set: DeviceSet,
    data: SP_DEVINFO_DATA,
}

impl NewRootDevice {
    /// Registers a root-enumerated device with hardware ID `Root\Viaduct`, in
    /// the INF's class, as Microsoft's devcon sample does (`devcon install`).
    fn create(inf: &Path) -> Result<Self, String> {
        let fail = |what: &str, e: windows::core::Error| {
            format!("creating the root device failed ({what}): {e}")
        };
        let mut class = GUID::zeroed();
        let mut class_name = [0u16; MAX_CLASS_NAME_LEN];
        // SAFETY: the class name buffer and its length are passed together.
        unsafe { SetupDiGetINFClassW(&HSTRING::from(inf), &mut class, &mut class_name, None) }
            .map_err(|e| fail("reading the INF's class", e))?;
        // SAFETY: a new, empty set for that class.
        let set = DeviceSet(
            unsafe { SetupDiCreateDeviceInfoList(Some(&class), None) }
                .map_err(|e| fail("device set", e))?,
        );
        let mut data = new_devinfo_data();
        // SAFETY: `class_name` is NUL-terminated by SetupDiGetINFClassW.
        unsafe {
            SetupDiCreateDeviceInfoW(
                set.0,
                PCWSTR(class_name.as_ptr()),
                &class,
                PCWSTR::null(),
                None,
                DICD_GENERATE_ID,
                Some(&mut data),
            )
        }
        .map_err(|e| fail("device information", e))?;
        let ids = to_multi_sz(&[ROOT_HARDWARE_ID]);
        // SAFETY: `data` belongs to `set`; the value is a REG_MULTI_SZ.
        unsafe {
            SetupDiSetDeviceRegistryPropertyW(set.0, &mut data, SPDRP_HARDWAREID, Some(&ids))
        }
        .map_err(|e| fail("hardware ID", e))?;
        // SAFETY: as above. Until this succeeds, dropping the set discards the device.
        unsafe { SetupDiCallClassInstaller(DIF_REGISTERDEVICE, set.0, Some(&data)) }
            .map_err(|e| fail("registering", e))?;
        println!("created the root device {}", set.instance_id(&data));
        Ok(NewRootDevice { set, data })
    }

    fn remove(self) {
        // SAFETY: the device was registered in this set by `create`.
        if unsafe { SetupDiCallClassInstaller(DIF_REMOVE, self.set.0, Some(&self.data)) }.is_ok() {
            println!("removed the root device again");
        }
    }
}

pub fn install(package: &str) -> Result<(), String> {
    require_admin()?;
    let inf = package_inf(package)?;

    let existing = {
        let set = DeviceSet::all()?;
        let found = set.with_hardware_id(ROOT_HARDWARE_ID);
        for data in &found {
            println!("root device already present: {}", set.instance_id(data));
        }
        found.len()
    };
    if existing > 1 {
        println!(
            "warning: {existing} root devices; run `viaduct-cli uninstall`, then install again, to keep one"
        );
    }
    let created = if existing == 0 {
        Some(NewRootDevice::create(&inf)?)
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
    match result {
        Ok(()) => println!("installed the driver from {}", inf.display()),
        // Not forced: an installed driver of the same or a newer version stays.
        Err(e) if created.is_none() && e.code() == HRESULT::from_win32(ERROR_NO_MORE_ITEMS.0) => {
            println!("the installed driver is the same version or newer; left as it is")
        }
        Err(e) => {
            if let Some(device) = created {
                device.remove();
            }
            return Err(format!(
                "installing the driver from {} failed: {e}",
                inf.display()
            ));
        }
    }
    if reboot.as_bool() {
        println!("Windows needs a reboot to finish.");
    }
    Ok(())
}

pub fn uninstall() -> Result<(), String> {
    require_admin()?;
    let mut reboot = false;
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
            reboot |= needs_reboot.as_bool();
            println!("removed the device {id}");
        }
    }
    let packages = viaduct_packages()?;
    if packages.is_empty() {
        println!("no Viaduct driver package in the driver store");
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
        println!("removed the driver package {name}");
    }
    if reboot {
        println!("Windows needs a reboot to finish (an app may still hold a Device).");
    }
    Ok(())
}

/// The published names (`oemN.inf`) of the driver store packages that install
/// `Root\Viaduct`.
fn viaduct_packages() -> Result<Vec<String>, String> {
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
