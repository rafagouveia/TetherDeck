use idevice::{
    IdeviceService, ReadWrite,
    provider::RsdProvider,
    provider::UsbmuxdProvider,
    services::{
        core_device::{ImageFormat, ScreenCaptureServiceClient},
        lockdown::LockdownClient,
        rsd::RsdHandshake,
        screenshotr::ScreenshotService,
    },
    usbmuxd::{Connection, UsbmuxdAddr},
};
use std::{net::IpAddr, time::Duration};
use thiserror::Error;

pub mod accessibility;
pub mod elements;
mod hid;
pub mod live;
pub mod video;
pub mod wifi;
pub mod wireless;
mod wireless_tunnel;
pub use hid::{perform_hid_rsd, perform_hid_usb, validate_hid_action};

pub use rust_ios_device_api::{API_VERSION, DeviceSummary, HidAction, IdentityStatus};

#[derive(Debug, Error)]
pub enum DeviceError {
    #[error("iOS protocol operation failed: {0}")]
    Idevice(#[from] idevice::IdeviceError),
    #[error("device {0} is no longer connected")]
    MissingDevice(String),
    #[error("RSD endpoint identifies a different device")]
    DeviceIdMismatch,
    #[error("device action result is unknown; inspect device state before retrying")]
    ActionOutcomeUnknown,
    #[error("element reference is stale; obtain a fresh snapshot")]
    StaleElement,
    #[error(
        "AX activation is restricted; start a signed WebDriverAgent runner for element control"
    )]
    ElementActivationUnsupported,
    #[error("WebDriverAgent is not responding on the configured device port")]
    WdaUnavailable,
    #[error("Wi-Fi WDA runner binding has not been reviewed for HTTP and MJPEG isolation")]
    WdaTunnelBindingReviewRequired,
    #[error(
        "network WDA requires the authenticated paired Wi-Fi tunnel, not a raw usbmux network port"
    )]
    WdaNetworkTunnelRequired,
    #[error("live screen frame is invalid or exceeds the supported size")]
    LiveFrameUnavailable,
    #[error(
        "remote Wi-Fi pairing record is missing; prepare or import a trusted record before opening developer services"
    )]
    WifiPairingRequired,
    #[error("WDA session was replaced; obtain fresh elements")]
    ElementSessionLost,
    #[error("this element does not expose a supported activation action")]
    ElementNotActionable,
    #[error("AX operation failed during {stage} ({kind})")]
    AxProviderFailed {
        stage: &'static str,
        kind: &'static str,
    },
    #[error("WDA operation failed during {stage}")]
    ElementProviderFailed {
        stage: &'static str,
        #[source]
        source: idevice::IdeviceError,
    },
    #[error("request timed out")]
    Timeout,
    #[error("RSD transport failed: {0}")]
    RsdTransport(#[from] std::io::Error),
    #[error("invalid RSD IP address")]
    InvalidRsdAddress,
    #[error("invalid USBMUXD_SOCKET_ADDRESS: {0}")]
    InvalidUsbmuxdAddress(#[from] std::net::AddrParseError),
    #[error("HID operation failed: {0}")]
    Hid(#[from] idevice::services::core_device::hid::MainKeyboardError),
    #[error("HID press or drag duration must be between 1 and 60000 ms")]
    InvalidHidDuration,
    #[error("live touch gesture identity or sequence is invalid")]
    InvalidTouchSequence,
    #[error("another live touch gesture is active")]
    TouchBusy,
    #[error("live touch coordinates are outside the physical screen")]
    InvalidTouchCoordinates,
    #[error("text contains a character unsupported by the US keyboard layout")]
    UnsupportedText,
    #[error("text must not exceed 4096 bytes")]
    InvalidTextLength,
    #[error("USB CoreDevice tunnel did not advertise an RSD service port")]
    MissingRsdPort,
}

impl DeviceError {
    /// Safe transport diagnostic for logs; never includes request data or protocol payloads.
    pub fn io_kind(&self) -> Option<std::io::ErrorKind> {
        match self {
            Self::RsdTransport(error) => Some(error.kind()),
            Self::Idevice(idevice::IdeviceError::Socket(error)) => Some(error.kind()),
            Self::ElementProviderFailed {
                source: idevice::IdeviceError::Socket(error),
                ..
            } => Some(error.kind()),
            _ => None,
        }
    }
}

fn provider(device_id: u32, udid: String) -> Result<UsbmuxdProvider, DeviceError> {
    Ok(UsbmuxdProvider {
        addr: UsbmuxdAddr::from_env_var()?,
        tag: 1,
        udid: udid.clone(),
        device_id,
        label: format!("rust-ios-device/{udid}"),
    })
}

pub async fn list_devices() -> Result<Vec<DeviceSummary>, DeviceError> {
    if wireless::id().is_some() {
        return wireless::summary().await.map(|d| vec![d]);
    }
    tokio::time::timeout(Duration::from_secs(8), async {
        let devices = tokio::time::timeout(Duration::from_secs(3), async {
            let mut mux = UsbmuxdAddr::from_env_var()?.connect(0).await?;
            mux.get_devices().await.map_err(DeviceError::from)
        })
        .await
        .map_err(|_| DeviceError::Timeout)??;
        let mut summaries = Vec::with_capacity(devices.len());
        let mut tasks = tokio::task::JoinSet::new();
        for device in devices.into_iter().filter(wifi::accepts) {
            tasks.spawn(async move {
                let p = provider(device.device_id, device.udid.clone())?;
                // A single unresponsive Lockdown service must not erase the USB inventory.
                let identity = tokio::time::timeout(Duration::from_secs(2), async {
                    let mut lockdown = LockdownClient::connect(&p).await?;
                    lockdown.get_value(None, None).await
                })
                .await;
                let (identity_status, name, product_type, product_version, build_version) =
                    match identity {
                        Ok(Ok(values)) => {
                            let get = |key: &str| {
                                values
                                    .as_dictionary()
                                    .and_then(|d| d.get(key))
                                    .and_then(|v| v.as_string())
                                    .map(str::to_owned)
                            };
                            (
                                IdentityStatus::Available,
                                get("DeviceName"),
                                get("ProductType"),
                                get("ProductVersion"),
                                get("BuildVersion"),
                            )
                        }
                        Ok(Err(error)) => (classify_identity_error(&error), None, None, None, None),
                        Err(_) => (IdentityStatus::ServiceUnavailable, None, None, None, None),
                    };
                let transport = match device.connection_type {
                    Connection::Usb => "usb".to_string(),
                    Connection::Network(ip) => format!("network:{ip}"),
                    Connection::Unknown(s) => format!("unknown:{s}"),
                };
                Ok::<_, DeviceError>(DeviceSummary {
                    device_id: device.udid.clone(),
                    udid: Some(device.udid),
                    transport,
                    identity_status,
                    name,
                    product_type,
                    product_version,
                    build_version,
                })
            });
        }
        while let Some(result) = tasks.join_next().await {
            summaries.push(result.map_err(|_| DeviceError::Timeout)??);
        }
        summaries.sort_by(|a, b| {
            a.device_id
                .cmp(&b.device_id)
                .then(a.transport.cmp(&b.transport))
        });
        Ok(summaries)
    })
    .await
    .map_err(|_| DeviceError::Timeout)?
}

pub async fn rsd_device(host: &str, port: u16) -> Result<DeviceSummary, DeviceError> {
    let addr: IpAddr = host.parse().map_err(|_| DeviceError::InvalidRsdAddress)?;
    tokio::time::timeout(Duration::from_secs(8), async {
        let stream = tokio::net::TcpStream::connect((addr, port)).await?;
        let handshake = RsdHandshake::new(stream).await?;
        let get = |key: &str| {
            handshake
                .properties
                .get(key)
                .and_then(|v| v.as_string())
                .map(str::to_owned)
        };
        Ok(DeviceSummary {
            device_id: handshake.uuid,
            udid: get("UniqueDeviceID"),
            transport: format!("rsd:{addr}:{port}"),
            identity_status: IdentityStatus::Available,
            name: get("DeviceName"),
            product_type: get("ProductType"),
            product_version: get("ProductVersion"),
            build_version: get("BuildVersion"),
        })
    })
    .await
    .map_err(|_| DeviceError::Timeout)?
}

fn classify_identity_error(error: &idevice::IdeviceError) -> IdentityStatus {
    match error {
        idevice::IdeviceError::InvalidHostID => IdentityStatus::Unpaired,
        idevice::IdeviceError::DeviceLocked => IdentityStatus::Locked,
        idevice::IdeviceError::UnknownErrorType(message)
            if matches!(message.as_str(), "PasswordProtected" | "DeviceLocked") =>
        {
            IdentityStatus::Locked
        }
        _ => IdentityStatus::ServiceUnavailable,
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    #[test]
    fn device_reported_errors_are_not_guessed_from_substrings() {
        assert_eq!(
            classify_identity_error(&idevice::IdeviceError::InvalidHostID),
            IdentityStatus::Unpaired
        );
        assert_eq!(
            classify_identity_error(&idevice::IdeviceError::DeviceLocked),
            IdentityStatus::Locked
        );
        assert_eq!(
            classify_identity_error(&idevice::IdeviceError::UnknownErrorType(
                "PasswordProtected".into()
            )),
            IdentityStatus::Locked
        );
        assert_eq!(
            classify_identity_error(&idevice::IdeviceError::UnknownErrorType(
                "NotDeviceLocked".into()
            )),
            IdentityStatus::ServiceUnavailable
        );
    }
}

pub(crate) async fn usb_rsd_stack(
    udid: String,
) -> Result<(idevice::tcp::handle::AdapterHandle, RsdHandshake), DeviceError> {
    if wireless::id().is_some() {
        return Box::pin(wireless_tunnel::connect(&udid)).await;
    }
    let lockdown_provider = wireless::provider(&udid).await?;
    tracing::info!("CoreDevice provider verified; opening proxy");
    let proxy =
        idevice::services::core_device_proxy::CoreDeviceProxy::connect(lockdown_provider.as_ref())
            .await?;
    tracing::info!("CoreDevice proxy tunnel established");
    let rsd_port = proxy.tunnel_info().server_rsd_port;
    if rsd_port == 0 {
        return Err(DeviceError::MissingRsdPort);
    }
    let adapter = proxy.create_software_tunnel()?;
    let mut handle = adapter.to_async_handle();
    let rsd_stream = handle.connect_to_service_port(rsd_port).await?;
    tracing::info!("CoreDevice RSD socket connected");
    let handshake = RsdHandshake::new(rsd_stream).await?;
    tracing::info!("CoreDevice RSD handshake completed");
    Ok((handle, handshake))
}

pub async fn screenshot_png_coredevice(udid: String) -> Result<Vec<u8>, DeviceError> {
    tokio::time::timeout(Duration::from_secs(30), async {
        let (mut provider, mut handshake) = usb_rsd_stack(udid).await?;
        type CaptureClient = ScreenCaptureServiceClient<Box<dyn ReadWrite>>;
        let mut capture: CaptureClient = handshake.connect(&mut provider).await?;
        Ok(capture.take_screenshot(None, ImageFormat::Png).await?)
    })
    .await
    .map_err(|_| DeviceError::Timeout)?
}

pub fn launch_app_coredevice(udid: String, bundle_id: String) -> Result<u32, DeviceError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(DeviceError::RsdTransport)?;
    runtime.block_on(async move {
        tokio::time::timeout(Duration::from_secs(30), async {
            let (mut provider, mut handshake) = usb_rsd_stack(udid).await?;
            type AppClient = idevice::services::core_device::AppServiceClient<Box<dyn ReadWrite>>;
            let mut app: AppClient = handshake.connect(&mut provider).await?;
            let launched = app
                .launch_application(bundle_id, &[], false, false, None, None, None)
                .await
                .map_err(|_| DeviceError::ActionOutcomeUnknown)?;
            Ok(launched.pid)
        })
        .await
        .map_err(|_| DeviceError::ActionOutcomeUnknown)?
    })
}

pub fn screenshot_png_rsd(
    addr: IpAddr,
    port: u16,
    expected_device_id: String,
) -> Result<Vec<u8>, DeviceError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(DeviceError::RsdTransport)?;
    runtime.block_on(async move {
        tokio::time::timeout(Duration::from_secs(20), async {
            let stream = tokio::net::TcpStream::connect((addr, port)).await?;
            let mut handshake = RsdHandshake::new(stream).await?;
            if handshake.uuid != expected_device_id {
                return Err(DeviceError::DeviceIdMismatch);
            }
            let mut provider = addr;
            type CaptureClient = ScreenCaptureServiceClient<Box<dyn ReadWrite + 'static>>;
            let mut capture: CaptureClient =
                handshake.connect::<CaptureClient>(&mut provider).await?;
            let png = capture.take_screenshot(None, ImageFormat::Png).await?;
            Ok(png)
        })
        .await
        .map_err(|_| DeviceError::Timeout)?
    })
}

pub fn launch_app_rsd(
    addr: IpAddr,
    port: u16,
    expected_device_id: String,
    bundle_id: String,
) -> Result<u32, DeviceError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(DeviceError::RsdTransport)?;
    runtime.block_on(async move {
        tokio::time::timeout(Duration::from_secs(20), async {
            let stream = tokio::net::TcpStream::connect((addr, port)).await?;
            let mut handshake = RsdHandshake::new(stream).await?;
            if handshake.uuid != expected_device_id {
                return Err(DeviceError::DeviceIdMismatch);
            }
            let mut provider = addr;
            type AppClient = idevice::services::core_device::AppServiceClient<Box<dyn ReadWrite>>;
            let mut app: AppClient = handshake.connect::<AppClient>(&mut provider).await?;
            let launched = app
                .launch_application(bundle_id, &[], false, false, None, None, None)
                .await
                .map_err(|_| DeviceError::ActionOutcomeUnknown)?;
            Ok(launched.pid)
        })
        .await
        .map_err(|_| DeviceError::ActionOutcomeUnknown)?
    })
}

pub fn screenshot_png(udid: String) -> Result<Vec<u8>, DeviceError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(DeviceError::RsdTransport)?;
    runtime.block_on(async move {
        if wireless::id().is_some() {
            return screenshot_png_coredevice(udid).await;
        }
        let legacy: Result<Vec<u8>, DeviceError> =
            tokio::time::timeout(Duration::from_secs(20), async {
                let mut mux = UsbmuxdAddr::from_env_var()?.connect(0).await?;
                let device = wifi::selected_device(&mut mux, &udid)
                    .await
                    .map_err(|error| match error {
                        idevice::IdeviceError::DeviceNotFound => {
                            DeviceError::MissingDevice(udid.clone())
                        }
                        other => DeviceError::Idevice(other),
                    })?;
                let p = provider(device.device_id, udid.clone())?;
                let mut screenshot = ScreenshotService::connect(&p).await?;
                Ok(screenshot.take_screenshot().await?)
            })
            .await
            .map_err(|_| DeviceError::Timeout)?;
        match legacy {
            Ok(png) => Ok(png),
            Err(_) => screenshot_png_coredevice(udid).await,
        }
    })
}
