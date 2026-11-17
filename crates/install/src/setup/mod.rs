//! The SetupAPI half of the install engine (ADR 0016, ADR 0024): the calls that
//! change the machine. Checked in the test VM (VM-STEPS D.5 and R), never run on
//! the host, and left out of coverage for that reason (README, Coverage). What
//! each step ends as is decided by the tested `install_outcome` and
//! `uninstall_outcome`.

use super::{
    DriverUpdate, DriverVersion, ENDPOINT_HARDWARE_ID, Outcome, ROOT_HARDWARE_ID, classify_kept,
    decode_inf, has_hardware_id, inf_driver_version, inf_lists_hardware_id, install_outcome,
    parse_driver_version, parse_multi_sz, restart_pending, to_multi_sz, uninstall_outcome,
};
use std::path::{Path, PathBuf};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_DEVNODE_STATUS_FLAGS, CM_Get_DevNode_Status, CM_PROB, CR_SUCCESS, DICD_GENERATE_ID,
    DIF_REGISTERDEVICE, DIF_REMOVE, DIGCF_ALLCLASSES, DN_HAS_PROBLEM, DiUninstallDevice, HDEVINFO,
    SP_DEVINFO_DATA, SPDRP_HARDWAREID, SUOI_FORCEDELETE, SetupDiCallClassInstaller,
    SetupDiCreateDeviceInfoList, SetupDiCreateDeviceInfoW, SetupDiDestroyDeviceInfoList,
    SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW,
    SetupDiGetDevicePropertyW, SetupDiGetDeviceRegistryPropertyW, SetupDiGetINFClassW,
    SetupDiSetDeviceRegistryPropertyW, SetupSetNonInteractiveMode, SetupUninstallOEMInfW,
    UPDATEDRIVERFORPLUGANDPLAYDEVICES_FLAGS, UpdateDriverForPlugAndPlayDevicesW,
};
use windows::Win32::Devices::Properties::{DEVPKEY_Device_DriverVersion, DEVPROPTYPE};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_INSUFFICIENT_BUFFER, ERROR_NO_MORE_ITEMS, HANDLE, HWND,
};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::{BOOL, GUID, HRESULT, HSTRING, PCWSTR};

mod root_device;
mod uninstall;

pub use root_device::{install, root_device_present};
pub use uninstall::uninstall;

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
