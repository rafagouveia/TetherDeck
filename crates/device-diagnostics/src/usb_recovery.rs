//! Windows recovery targets come from a present Apple USB parent and its children.
//! No wildcard resets, driver replacement, pairing deletion or automatic reboot.
use rust_ios_device_api::UsbRecoveryTarget;

pub async fn inventory() -> Result<Vec<UsbRecoveryTarget>, String> {
    #[cfg(windows)]
    {
        tokio::time::timeout(
            std::time::Duration::from_secs(4),
            tokio::task::spawn_blocking(platform::inventory),
        )
        .await
        .map_err(|_| "usb_inventory_timeout".to_owned())?
        .map_err(|_| "usb_inventory_worker_failed".to_owned())?
    }
    #[cfg(not(windows))]
    {
        Ok(Vec::new())
    }
}

pub async fn restart(device_id: &str, instance_id: &str) -> Result<&'static str, String> {
    #[cfg(windows)]
    {
        let targets = inventory().await?;
        if targets
            .iter()
            .filter(|target| target.device_id == device_id && target.instance_id == instance_id)
            .count()
            != 1
        {
            return Err("usb_target_not_present".into());
        }
        let mut command = tokio::process::Command::new(platform::pnputil()?);
        command
            .args(["/restart-device", instance_id])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .creation_flags(0x08000000);
        let status = tokio::time::timeout(std::time::Duration::from_secs(20), command.status())
            .await
            .map_err(|_| "usb_restart_outcome_unknown".to_owned())?
            .map_err(|_| "usb_restart_not_started".to_owned())?;
        match status.code() {
            Some(0) => Ok("completed"),
            Some(3010) => Ok("reboot_required"),
            Some(5) => Err("permission_required".into()),
            _ => Err("usb_restart_failed".into()),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (device_id, instance_id);
        Err("usb_recovery_platform_unsupported".into())
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use nusb::MaybeFuture;
    use std::{os::windows::ffi::OsStrExt, path::PathBuf};
    use windows_sys::Win32::{
        Devices::{DeviceAndDriverInstallation::*, Properties::*},
        Foundation::DEVPROPKEY,
        System::SystemInformation::GetSystemDirectoryW,
    };

    fn property(node: u32, key: &DEVPROPKEY) -> Option<String> {
        let mut buffer = [0u16; 1024];
        let mut kind = 0;
        let mut bytes = std::mem::size_of_val(&buffer) as u32;
        // Buffer is aligned for UTF-16 and its byte capacity is passed to Windows.
        let result = unsafe {
            CM_Get_DevNode_PropertyW(
                node,
                key,
                &mut kind,
                buffer.as_mut_ptr().cast(),
                &mut bytes,
                0,
            )
        };
        if result != CR_SUCCESS
            || kind != DEVPROP_TYPE_STRING
            || bytes as usize > std::mem::size_of_val(&buffer)
        {
            return None;
        }
        let end = buffer.iter().position(|c| *c == 0)?;
        String::from_utf16(&buffer[..end]).ok()
    }
    fn node_id(node: u32) -> Option<String> {
        let mut buffer = [0u16; 1024];
        if unsafe { CM_Get_Device_IDW(node, buffer.as_mut_ptr(), buffer.len() as u32, 0) }
            != CR_SUCCESS
        {
            return None;
        }
        String::from_utf16(&buffer[..buffer.iter().position(|c| *c == 0)?]).ok()
    }
    fn power(node: u32) -> Option<String> {
        let mut buffer = [0u32; 14];
        let mut kind = 0;
        let mut bytes = std::mem::size_of_val(&buffer) as u32;
        if unsafe {
            CM_Get_DevNode_PropertyW(
                node,
                &DEVPKEY_Device_PowerData,
                &mut kind,
                buffer.as_mut_ptr().cast(),
                &mut bytes,
                0,
            )
        } != CR_SUCCESS
            || kind != DEVPROP_TYPE_BINARY
            || bytes < 8
        {
            return None;
        }
        match buffer[1] {
            1..=4 => Some(format!("D{}", buffer[1] - 1)),
            _ => None,
        }
    }
    pub fn inventory() -> Result<Vec<UsbRecoveryTarget>, String> {
        let devices = nusb::list_devices()
            .wait()
            .map_err(|_| "usb_inventory_unavailable".to_owned())?;
        let mut targets = Vec::new();
        for device in devices.filter(|d| d.vendor_id() == 0x05ac) {
            let parent = device.instance_id().to_string_lossy().into_owned();
            let upper = parent.to_ascii_uppercase();
            if !upper.starts_with("USB\\VID_05AC&PID_") || upper.contains("&MI_") {
                continue;
            }
            let Some(serial) = upper.rsplit('\\').next() else {
                continue;
            };
            if serial.len() < 16 || !serial.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            let id = if serial.len() == 24 {
                format!("{}-{}", &serial[..8], &serial[8..])
            } else {
                serial.into()
            };
            let mut wide: Vec<_> = device.instance_id().encode_wide().chain(Some(0)).collect();
            let mut node = 0;
            if unsafe { CM_Locate_DevNodeW(&mut node, wide.as_mut_ptr(), CM_LOCATE_DEVNODE_NORMAL) }
                != CR_SUCCESS
            {
                continue;
            }
            let mut child = 0;
            if unsafe { CM_Get_Child(&mut child, node, 0) } != CR_SUCCESS {
                continue;
            }
            // Composite USB devices have few interfaces; a hard bound protects malformed topology.
            for _ in 0..32 {
                if let Some(instance) = node_id(child) {
                    let instance_upper = instance.to_ascii_uppercase();
                    let service = property(child, &DEVPKEY_Device_Service).unwrap_or_default();
                    if instance_upper.starts_with("USB\\VID_05AC&PID_")
                        && instance_upper.contains("&MI_")
                        && property(child, &DEVPKEY_Device_Class)
                            .is_some_and(|c| c.eq_ignore_ascii_case("USBDevice"))
                        && matches!(
                            service.to_ascii_lowercase().as_str(),
                            "winusb" | "usbaapl64" | "usbaapl"
                        )
                    {
                        let mut status = 0;
                        let mut problem = 0;
                        let result =
                            unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, child, 0) };
                        targets.push(UsbRecoveryTarget {
                            device_id: id.clone(),
                            instance_id: instance,
                            parent_instance_id: parent.clone(),
                            driver_service: service,
                            driver_provider: property(child, &DEVPKEY_Device_DriverProvider),
                            driver_version: property(child, &DEVPKEY_Device_DriverVersion),
                            driver_inf: property(child, &DEVPKEY_Device_DriverInfPath),
                            problem_code: (result == CR_SUCCESS).then_some(problem),
                            power_state: power(child),
                        });
                    }
                }
                let mut next = 0;
                if unsafe { CM_Get_Sibling(&mut next, child, 0) } != CR_SUCCESS {
                    break;
                }
                child = next;
            }
        }
        targets.sort_by(|a, b| a.instance_id.cmp(&b.instance_id));
        targets.dedup_by(|a, b| a.instance_id == b.instance_id);
        Ok(targets)
    }
    pub fn pnputil() -> Result<PathBuf, String> {
        let mut buffer = [0u16; 1024];
        let size =
            unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
        if size == 0 || size >= buffer.len() {
            return Err("system_directory_unavailable".into());
        }
        let path = String::from_utf16(&buffer[..size])
            .map_err(|_| "system_directory_unavailable".to_owned())?;
        Ok(PathBuf::from(path).join("pnputil.exe"))
    }
}
