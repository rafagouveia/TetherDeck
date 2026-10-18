use rust_ios_device_api::ServiceInfo;

pub fn service_name() -> &'static str {
    if cfg!(windows) {
        "Apple Mobile Device Service"
    } else {
        "usbmuxd.service"
    }
}

fn unknown(code: &str) -> ServiceInfo {
    ServiceInfo {
        name: service_name().into(),
        state: "unknown".into(),
        process_id: None,
        code: code.into(),
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        ffi::OsStr,
        time::{Duration, Instant},
    };
    use windows_service::{
        service::{ServiceAccess, ServiceState},
        service_manager::{ServiceManager, ServiceManagerAccess},
    };

    fn classify(error: windows_service::Error) -> String {
        match error {
            windows_service::Error::Winapi(error) => match error.raw_os_error() {
                Some(5) => "permission_required",
                Some(1060) => "service_not_installed",
                _ => "service_control_failed",
            }
            .into(),
            _ => "service_control_failed".into(),
        }
    }

    pub async fn snapshot() -> ServiceInfo {
        tokio::task::spawn_blocking(|| {
            let result = (|| {
                let manager =
                    ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
                let service = manager.open_service(service_name(), ServiceAccess::QUERY_STATUS)?;
                service.query_status()
            })();
            match result {
                Ok(status) => ServiceInfo {
                    name: service_name().into(),
                    state: format!("{:?}", status.current_state).to_lowercase(),
                    process_id: status.process_id,
                    code: "service_queried".into(),
                },
                Err(error) => unknown(&classify(error)),
            }
        })
        .await
        .unwrap_or_else(|_| unknown("service_query_failed"))
    }

    pub async fn restart() -> Result<(), String> {
        tokio::task::spawn_blocking(|| {
            let manager =
                ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
                    .map_err(classify)?;
            let service = manager
                .open_service(
                    service_name(),
                    ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::START,
                )
                .map_err(classify)?;
            let deadline = Instant::now() + Duration::from_secs(20);
            let current = service.query_status().map_err(classify)?.current_state;
            if matches!(
                current,
                ServiceState::StartPending
                    | ServiceState::ContinuePending
                    | ServiceState::PausePending
            ) {
                return Err("service_transition_in_progress".into());
            }
            if current != ServiceState::Stopped {
                if current != ServiceState::StopPending {
                    service.stop().map_err(classify)?;
                }
                loop {
                    if service.query_status().map_err(classify)?.current_state
                        == ServiceState::Stopped
                    {
                        break;
                    }
                    if Instant::now() >= deadline {
                        return Err("service_stop_timeout".into());
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
            service.start::<&OsStr>(&[]).map_err(classify)?;
            loop {
                if service.query_status().map_err(classify)?.current_state == ServiceState::Running
                {
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    return Err("service_start_timeout".into());
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        })
        .await
        .map_err(|_| "service_worker_failed".to_owned())?
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::time::Duration;

    async fn systemctl(args: &[&str], seconds: u64) -> Result<std::process::Output, String> {
        let mut command = tokio::process::Command::new("systemctl");
        command.args(args).kill_on_drop(true);
        tokio::time::timeout(Duration::from_secs(seconds), command.output())
            .await
            .map_err(|_| "service_control_timeout".to_owned())?
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    "service_manager_unavailable"
                } else {
                    "service_control_failed"
                }
                .to_owned()
            })
    }

    pub async fn snapshot() -> ServiceInfo {
        match systemctl(
            &[
                "show",
                "usbmuxd.service",
                "--property=ActiveState,MainPID,LoadState",
            ],
            3,
        )
        .await
        {
            Ok(output) if output.status.success() => {
                let text = String::from_utf8_lossy(&output.stdout);
                let get = |key: &str| {
                    text.lines()
                        .find_map(|line| line.strip_prefix(key))
                        .map(str::to_owned)
                };
                let loaded = get("LoadState=").as_deref() == Some("loaded");
                ServiceInfo {
                    name: service_name().into(),
                    state: get("ActiveState=").unwrap_or_else(|| "unknown".into()),
                    process_id: get("MainPID=")
                        .and_then(|p| p.parse::<u32>().ok())
                        .filter(|p| *p != 0),
                    code: if loaded {
                        "service_queried"
                    } else {
                        "service_not_installed"
                    }
                    .into(),
                }
            }
            Ok(_) => unknown("service_manager_unavailable"),
            Err(code) => unknown(&code),
        }
    }

    pub async fn restart() -> Result<(), String> {
        let output = systemctl(&["--no-ask-password", "restart", "usbmuxd.service"], 20).await?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("Access denied")
            || stderr.contains("authentication")
            || stderr.contains("Permission")
        {
            Err("permission_required".into())
        } else {
            Err("service_control_failed".into())
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    use super::*;
    pub async fn snapshot() -> ServiceInfo {
        unknown("platform_not_supported")
    }
    pub async fn restart() -> Result<(), String> {
        Err("platform_not_supported".into())
    }
}

pub use platform::{restart, snapshot};
