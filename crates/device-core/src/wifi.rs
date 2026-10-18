//! Paired Wi-Fi setup and an immutable per-process transport choice.
use crate::*;
use idevice::{
    provider::IdeviceProvider,
    usbmuxd::{UsbmuxdConnection, UsbmuxdDevice},
};
use std::sync::OnceLock;

static TRANSPORT: OnceLock<String> = OnceLock::new();
pub fn configure_transport(value: &str) -> Result<(), &'static str> {
    if !matches!(value, "auto" | "usb" | "wifi") {
        return Err("invalid transport");
    }
    TRANSPORT
        .set(value.into())
        .map_err(|_| "transport already configured")
}
pub(crate) fn accepts(device: &UsbmuxdDevice) -> bool {
    match TRANSPORT.get().map(String::as_str).unwrap_or("auto") {
        "usb" => matches!(device.connection_type, Connection::Usb),
        "wifi" => matches!(device.connection_type, Connection::Network(_)),
        _ => true,
    }
}
pub(crate) async fn selected_device(
    mux: &mut UsbmuxdConnection,
    id: &str,
) -> Result<UsbmuxdDevice, idevice::IdeviceError> {
    mux.get_devices()
        .await?
        .into_iter()
        .find(|d| d.udid == id && accepts(d))
        .ok_or(idevice::IdeviceError::DeviceNotFound)
}
/// Explicit user action. Requires an existing trusted USB pair; never initiates pairing.
/// Readback confirms only the device setting, not network reachability.
pub async fn enable(id: &str) -> Result<(), DeviceError> {
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut mux = UsbmuxdAddr::from_env_var()?.connect(0).await?;
        let device = mux
            .get_devices()
            .await?
            .into_iter()
            .find(|d| d.udid == id && matches!(d.connection_type, Connection::Usb))
            .ok_or_else(|| DeviceError::MissingDevice(id.into()))?;
        let provider = super::provider(device.device_id, device.udid)?;
        let pair = provider.get_pairing_file().await?;
        let mut lockdown = LockdownClient::connect(&provider).await?;
        lockdown.start_session(&pair).await?;
        let domain = Some("com.apple.mobile.wireless_lockdown");
        lockdown
            .set_value("EnableWifiConnections", true.into(), domain)
            .await?;
        if lockdown
            .get_value(Some("EnableWifiConnections"), domain)
            .await?
            .as_boolean()
            != Some(true)
        {
            return Err(DeviceError::Idevice(
                idevice::IdeviceError::UnexpectedResponse("Wi-Fi setting not confirmed".into()),
            ));
        }
        Ok(())
    })
    .await
    .map_err(|_| DeviceError::ActionOutcomeUnknown)?
}
