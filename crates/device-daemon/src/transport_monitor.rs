//! Read-only mux enumeration records bounded transitions. It never restarts or replays actions.
use super::*;
use idevice::usbmuxd::UsbmuxdAddr;
use rust_ios_device_api::TransportObservation;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub async fn run(state: AppState) {
    {
        let mut history = state.transport_history.lock().await;
        history.started_at_unix_ms = now();
        history.sample_interval_ms = 5000;
    }
    let mut previous: Option<Result<Vec<(String, u32)>, &'static str>> = None;
    let mut invalidate = false;
    loop {
        let result = tokio::time::timeout(Duration::from_secs(3), async {
            let mut mux = UsbmuxdAddr::from_env_var()
                .map_err(|_| "mux_address_invalid")?
                .connect(0)
                .await
                .map_err(|_| "mux_endpoint_unavailable")?;
            let devices = mux.get_devices().await.map_err(|_| "mux_protocol_failed")?;
            let mut devices: Vec<_> = devices
                .into_iter()
                .filter(|d| state.device.as_ref().is_none_or(|id| id.as_str() == d.udid))
                .map(|d| (d.udid, d.device_id))
                .collect();
            devices.sort();
            Ok(devices)
        })
        .await
        .unwrap_or(Err("mux_probe_timeout"));
        {
            let mut history = state.transport_history.lock().await;
            history.samples = history.samples.saturating_add(1);
            history.last_sample_at_unix_ms = now();
            if previous.as_ref() != Some(&result) {
                invalidate = previous.is_some();
                let (code, count) = match &result {
                    Ok(devices) => (
                        if devices.is_empty() {
                            "scoped_devices_absent"
                        } else {
                            "scoped_devices_present"
                        },
                        devices.len(),
                    ),
                    Err(code) => (*code, 0),
                };
                history.observations.push(TransportObservation {
                    observed_at_unix_ms: now(),
                    code: code.into(),
                    device_count: count,
                });
                if history.observations.len() > 32 {
                    history.observations.remove(0);
                }
            }
        }
        // Never wait behind a device action. If busy, invalidate after the next sample.
        if invalidate {
            if let Ok(mut sessions) = state.elements.try_lock() {
                sessions.clear();
                invalidate = false;
            }
        }
        previous = Some(result);
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
