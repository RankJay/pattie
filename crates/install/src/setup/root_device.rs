//! The Pattie root device: making it, removing it, and reading its driver version
//! and problem codes.

use super::*;

mod install;

pub use install::install;

/// MAX_CLASS_NAME_LEN in setupapi.h.
const MAX_CLASS_NAME_LEN: usize = 32;

/// A root device this run registered, removed again if the install fails.
struct NewRootDevice {
    set: DeviceSet,
    data: SP_DEVINFO_DATA,
}

impl NewRootDevice {
    /// Registers a root-enumerated device with hardware ID `Root\Pattie`, in
    /// the INF's class, as Microsoft's devcon sample does (`devcon install`).
    fn create(inf: &Path, log: &mut dyn FnMut(&str)) -> Result<Self, String> {
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
        log(&format!(
            "created the root device {}",
            set.instance_id(&data)
        ));
        Ok(NewRootDevice { set, data })
    }

    fn remove(self, log: &mut dyn FnMut(&str)) {
        // SAFETY: the device was registered in this set by `create`.
        if unsafe { SetupDiCallClassInstaller(DIF_REMOVE, self.set.0, Some(&self.data)) }.is_ok() {
            log("removed the root device again");
        }
    }
}

/// Whether a Pattie root device exists: the MSI asks before installing, so a
/// failed install doesn't roll back a driver `pattie-cli install` put there.
pub fn root_device_present() -> Result<bool, String> {
    Ok(!DeviceSet::all()?
        .with_hardware_id(ROOT_HARDWARE_ID)
        .is_empty())
}

/// The lowest driver version among the root devices that have one; None when
/// none does (a root with no driver installed reads as no version). A leftover
/// root without a driver beside the real one doesn't hide the real one's.
fn root_driver_version() -> Result<Option<DriverVersion>, String> {
    let set = DeviceSet::all()?;
    Ok(set
        .with_hardware_id(ROOT_HARDWARE_ID)
        .iter()
        .filter_map(|data| set.driver_version(data))
        .min())
}

/// The problem codes of the root devices that have one (Device Manager's
/// "Error" state), read after the update.
fn root_problems() -> Result<Vec<u32>, String> {
    let set = DeviceSet::all()?;
    let mut problems = Vec::new();
    for data in set.with_hardware_id(ROOT_HARDWARE_ID) {
        let mut status = CM_DEVNODE_STATUS_FLAGS(0);
        let mut problem = CM_PROB(0);
        // SAFETY: both out-pointers are locals alive for the call; DevInst came
        // from this device information set.
        let result = unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, data.DevInst, 0) };
        if result == CR_SUCCESS && (status.0 & DN_HAS_PROBLEM.0) != 0 {
            problems.push(problem.0);
        }
    }
    Ok(problems)
}

impl DeviceSet {
    /// The device's installed driver version (DEVPKEY_Device_DriverVersion).
    fn driver_version(&self, data: &SP_DEVINFO_DATA) -> Option<DriverVersion> {
        let mut buffer = [0u8; 128];
        let mut kind = DEVPROPTYPE(0);
        let mut needed = 0u32;
        // SAFETY: the buffer and its length are passed together; the key is a
        // static from the windows crate.
        unsafe {
            SetupDiGetDevicePropertyW(
                self.0,
                data,
                &DEVPKEY_Device_DriverVersion,
                &mut kind,
                Some(&mut buffer),
                Some(&mut needed),
                0,
            )
        }
        .ok()?;
        let units: Vec<u16> = buffer[..(needed as usize).min(buffer.len())]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|&unit| unit != 0)
            .collect();
        parse_driver_version(&String::from_utf16_lossy(&units))
    }
}
