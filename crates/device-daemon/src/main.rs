use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::Response,
    routing::get,
};
use clap::Parser;
use rust_ios_device_api::API_VERSION;
use rust_ios_device_core::{
    HidAction, launch_app_coredevice, launch_app_rsd, list_devices, perform_hid_rsd,
    perform_hid_usb, rsd_device, screenshot_png, screenshot_png_rsd, validate_hid_action,
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
};

mod control;
mod elements;
mod rsd_discovery;
mod screen;
mod transport_monitor;
mod video;

#[derive(Parser)]
#[command(about = "Local iOS device daemon; bind to loopback and use SSH for remote access")]
struct Args {
    #[arg(long, default_value = "127.0.0.1:18087")]
    listen: SocketAddr,
    #[arg(long, env = "RUST_IOS_DEVICE_TOKEN")]
    token: Option<String>,
    /// Print local host diagnostics as JSON and exit, even when the daemon is down.
    #[arg(long, conflicts_with = "recover_service")]
    doctor: bool,
    /// Explicitly restart the host's multiplexer service, diagnose, then exit.
    #[arg(long)]
    recover_service: bool,
    /// Explicit local recovery of one exact present Apple USB interface, then diagnose and exit.
    #[arg(long, requires = "device", conflicts_with_all = ["doctor", "recover_service", "rsd_host", "rsd_port", "tunneld_url", "wda_runner"])]
    recover_usb_interface: Option<String>,
    /// Permit authenticated, serialized service restarts with a 60-second cooldown.
    #[arg(long)]
    allow_service_restart: bool,
    /// Permit restart of an exact present Apple USB interface belonging to the scoped phone.
    #[arg(long, requires = "device")]
    allow_usb_interface_restart: bool,
    /// Restrict this daemon instance to one device identifier from the devices endpoint.
    #[arg(long)]
    device: Option<String>,
    /// Select usbmux USB or Wi-Fi entries; never silently fall back between them.
    #[arg(long, default_value = "auto", value_parser = ["auto", "usb", "wifi"])]
    transport: String,
    /// Enable paired Wi-Fi connections over an existing trusted USB session, then exit.
    #[arg(long, requires = "device", conflicts_with_all = ["doctor", "recover_service", "recover_usb_interface", "rsd_host", "rsd_port", "tunneld_url", "wda_runner"])]
    enable_wifi: bool,
    /// Connect directly to paired Lockdown over Wi-Fi; Bonjour discovery if no host is supplied.
    #[arg(long, requires = "device", conflicts_with_all = ["rsd_host", "tunneld_url", "enable_wifi"])]
    wifi_direct: bool,
    #[arg(long, requires = "wifi_direct")]
    wifi_host: Option<IpAddr>,
    /// Optional active RSD endpoint, configured from a trusted local tunnel.
    #[arg(long)]
    rsd_host: Option<IpAddr>,
    #[arg(long, requires = "rsd_host")]
    rsd_port: Option<u16>,
    /// Discover and refresh this scoped phone's RSD endpoint from a trusted local tunneld.
    #[arg(long, requires = "device", conflicts_with_all = ["rsd_host", "rsd_port"])]
    tunneld_url: Option<String>,
    /// Device-side WDA HTTP port (no host port forwarding required).
    #[arg(long, default_value_t = 8100, value_parser = clap::value_parser!(u16).range(1..))]
    wda_port: u16,
    /// Fail explicitly instead of using read-only AX when WDA is unavailable.
    #[arg(long)]
    require_wda: bool,
    /// Launch an installed, development-signed XCTest runner for --device.
    #[arg(long, requires = "device")]
    wda_runner: Option<String>,
    /// The exact --wda-runner was reviewed to bind HTTP AND MJPEG only to USE_IP (e.g. WDA 16.15.3).
    /// Required for paired Wi-Fi WDA; does not attest an arbitrary runner binary.
    #[arg(long, requires = "wda_runner")]
    wda_tunnel_binding_reviewed: bool,
}

#[derive(Clone)]
struct AppState {
    video_control: Arc<tokio::sync::Mutex<Option<video::LiveControl>>>,
    screen_gate: Arc<tokio::sync::Semaphore>,
    rsd_discovery: Option<Arc<rsd_discovery::Discovery>>,
    transport_history: Arc<tokio::sync::Mutex<rust_ios_device_api::TransportHistory>>,
    runner_bundle: Option<Arc<String>>,
    wda_tunnel_binding_reviewed: bool,
    runner: Arc<tokio::sync::Mutex<Option<rust_ios_device_core::elements::Runner>>>,
    runner_failure: Arc<tokio::sync::Mutex<Option<(&'static str, &'static str)>>>,
    wda_port: u16,
    require_wda: bool,
    elements: elements::Sessions,
    token: Arc<String>,
    device: Option<Arc<String>>,
    rsd: Option<(IpAddr, u16)>,
    gate: Arc<tokio::sync::Semaphore>,
    allow_service_restart: bool,
    allow_usb_interface_restart: bool,
    last_recovery: Arc<tokio::sync::Mutex<Option<std::time::Instant>>>,
}

fn is_allowed_rsd_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        IpAddr::V6(ip) => {
            let first = ip.segments()[0];
            ip.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

fn authorized(headers: &HeaderMap, state: &AppState) -> bool {
    let token_ok = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        == Some(state.token.as_str());
    let version_ok = headers
        .get("x-protocol-version")
        .and_then(|v| v.to_str().ok())
        == Some("1");
    token_ok && version_ok
}
async fn acquire_device(state: &AppState) -> Result<tokio::sync::OwnedSemaphorePermit, StatusCode> {
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        state.gate.clone().acquire_owned(),
    )
    .await
    .map_err(|_| StatusCode::LOCKED)?
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}

async fn health() -> Json<serde_json::Value> {
    Json(
        serde_json::json!({"api_version": API_VERSION, "service": "rust-ios-device", "state": "ready"}),
    )
}

async fn devices(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !authorized(&headers, &state) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let _permit = acquire_device(&state).await?;
    let mut devices = match list_devices().await {
        Ok(devices) => devices,
        Err(err) if state.has_rsd() => {
            tracing::warn!(error = %err, io_kind = ?err.io_kind(), "USB enumeration failed; checking configured RSD target");
            Vec::new()
        }
        Err(err) => {
            tracing::warn!(error = %err, io_kind = ?err.io_kind(), "device enumeration failed");
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
    };
    if let Some(target) = state.rsd_target().await {
        let identity = match target.identity {
            Some(device) => Ok(device),
            None => rsd_device(&target.host.to_string(), target.port).await,
        };
        match identity {
            Ok(device) => {
                if !devices.iter().any(|d| d.device_id == device.device_id) {
                    devices.push(device)
                }
            }
            Err(err) if devices.is_empty() => {
                tracing::warn!(error = %err, io_kind = ?err.io_kind(), "configured RSD target unavailable");
                return Err(StatusCode::SERVICE_UNAVAILABLE);
            }
            Err(err) => {
                tracing::warn!(error = %err, io_kind = ?err.io_kind(), "configured RSD target unavailable; retaining USB results")
            }
        }
    }
    if let Some(allowed) = &state.device {
        devices.retain(|device| &device.device_id == allowed.as_str());
    }
    Ok(Json(
        serde_json::json!({"api_version": API_VERSION, "devices": devices}),
    ))
}

async fn screenshot(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Result<Response, StatusCode> {
    if !authorized(&headers, &state) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if device_id.len() > 128
        || !device_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    if state
        .device
        .as_deref()
        .is_some_and(|allowed| allowed.as_str() != device_id)
    {
        return Err(StatusCode::FORBIDDEN);
    }
    let _permit = acquire_device(&state).await?;
    if let Some(result) = video::screenshot_if_active(&state, &device_id).await {
        return result
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
            .and_then(png_response);
    }
    if state.screen_gate.available_permits() == 0 {
        return Err(StatusCode::LOCKED);
    }
    let mut rsd_unavailable = state.rsd_discovery.is_some();
    if let Some(target) = state.rsd_target().await {
        let (host, port) = (target.host, target.port);
        let expected_device_id = target.expected_rsd_id.unwrap_or_else(|| device_id.clone());
        match tokio::task::spawn_blocking(move || {
            screenshot_png_rsd(host, port, expected_device_id)
        })
        .await
        {
            Ok(Ok(bytes)) => return png_response(bytes),
            Ok(Err(rust_ios_device_core::DeviceError::DeviceIdMismatch)) => {}
            Ok(Err(error)) => {
                tracing::warn!(error = %error, io_kind = ?error.io_kind(), "RSD screenshot failed; checking USB");
                rsd_unavailable = true;
            }
            Err(_) => rsd_unavailable = true,
        }
    }
    let devices = match list_devices().await {
        Ok(devices) => devices,
        Err(error) => {
            tracing::warn!(error = %error, io_kind = ?error.io_kind(), "USB enumeration failed for screenshot");
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
    };
    let Some(device) = devices
        .into_iter()
        .find(|device| device.device_id == device_id)
    else {
        return Err(if rsd_unavailable {
            StatusCode::SERVICE_UNAVAILABLE
        } else {
            StatusCode::NOT_FOUND
        });
    };
    let udid = device.udid.unwrap_or_else(|| device.device_id.clone());
    let result = match tokio::task::spawn_blocking(move || screenshot_png(udid)).await {
        Ok(result) => result,
        Err(_) => Err(rust_ios_device_core::DeviceError::Timeout),
    };
    result
        .map_err(|err| {
            tracing::warn!(error = %err, io_kind = ?err.io_kind(), "USB screenshot failed");
            StatusCode::SERVICE_UNAVAILABLE
        })
        .and_then(png_response)
}

fn png_response(bytes: Vec<u8>) -> Result<Response, StatusCode> {
    match bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        true => Ok(Response::builder()
            .header(header::CONTENT_TYPE, "image/png")
            .body(Body::from(bytes))
            .unwrap()),
        false => Err(StatusCode::UNSUPPORTED_MEDIA_TYPE),
    }
}

#[derive(serde::Deserialize)]
struct LaunchRequest {
    bundle_id: String,
}

async fn launch_app(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
    Json(request): Json<LaunchRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !authorized(&headers, &state) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if request.bundle_id.len() > 255
        || !request
            .bundle_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        || !request.bundle_id.contains('.')
        || device_id.len() > 128
        || !device_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    if state
        .device
        .as_deref()
        .is_some_and(|allowed| allowed.as_str() != device_id)
    {
        return Err(StatusCode::FORBIDDEN);
    }
    let _permit = acquire_device(&state).await?;
    if let Some(session) = state.elements.lock().await.get_mut(&device_id) {
        session.invalidate();
    }
    let mut rsd_failed = state.rsd_discovery.is_some();
    if let Some(target) = state.rsd_target().await {
        let (host, port) = (target.host, target.port);
        let expected_device_id = target.expected_rsd_id.unwrap_or_else(|| device_id.clone());
        let bundle_id = request.bundle_id.clone();
        match tokio::task::spawn_blocking(move || {
            launch_app_rsd(host, port, expected_device_id, bundle_id)
        })
        .await
        {
            Ok(Ok(pid)) => {
                return Ok(Json(
                    serde_json::json!({"device_id": device_id, "pid": pid}),
                ));
            }
            Ok(Err(rust_ios_device_core::DeviceError::DeviceIdMismatch)) => {}
            Ok(Err(rust_ios_device_core::DeviceError::ActionOutcomeUnknown)) => {
                return Err(StatusCode::SERVICE_UNAVAILABLE);
            }
            Ok(Err(error)) => {
                tracing::warn!(error = %error, io_kind = ?error.io_kind(), "RSD app launch failed; checking USB");
                rsd_failed = true;
            }
            // A worker panic can occur after dispatch. Do not replay on USB.
            Err(_) => return Err(StatusCode::SERVICE_UNAVAILABLE),
        }
    }
    let devices = list_devices().await.map_err(|error| {
        tracing::warn!(error = %error, io_kind = ?error.io_kind(), "USB enumeration failed for app launch");
        StatusCode::SERVICE_UNAVAILABLE
    })?;
    let Some(device) = devices
        .into_iter()
        .find(|device| device.device_id == device_id)
    else {
        return Err(if rsd_failed {
            StatusCode::SERVICE_UNAVAILABLE
        } else {
            StatusCode::NOT_FOUND
        });
    };
    let udid = device.udid.unwrap_or_else(|| device.device_id.clone());
    let bundle_id = request.bundle_id;
    let pid = tokio::task::spawn_blocking(move || launch_app_coredevice(udid, bundle_id))
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .map_err(|error| {
            tracing::warn!(error = %error, io_kind = ?error.io_kind(), "USB CoreDevice app launch failed");
            StatusCode::SERVICE_UNAVAILABLE
        })?;
    Ok(Json(
        serde_json::json!({"device_id": device_id, "pid": pid}),
    ))
}

async fn hid_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
    Json(action): Json<HidAction>,
) -> Result<StatusCode, StatusCode> {
    if !authorized(&headers, &state) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if device_id.len() > 128
        || !device_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || validate_hid_action(&action).is_err()
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    if state
        .device
        .as_deref()
        .is_some_and(|allowed| allowed.as_str() != device_id)
    {
        return Err(StatusCode::FORBIDDEN);
    }
    let _permit = acquire_device(&state).await?;
    if let Some(session) = state.elements.lock().await.get_mut(&device_id) {
        session.invalidate();
    }
    if let Some(result) = video::action_if_active(&state, &device_id, action.clone()).await {
        return result.map(|()| StatusCode::NO_CONTENT).map_err(|error| {
            tracing::warn!(error = %error, "live HID action failed; command will not be replayed");
            StatusCode::SERVICE_UNAVAILABLE
        });
    }
    if state.screen_gate.available_permits() == 0 {
        return Err(StatusCode::LOCKED);
    }
    let mut rsd_unavailable = state.rsd_discovery.is_some();
    if let Some(target) = state.rsd_target().await {
        let (host, port) = (target.host, target.port);
        let expected_device_id = target.expected_rsd_id.unwrap_or_else(|| device_id.clone());
        let rsd_action = action.clone();
        match tokio::task::spawn_blocking(move || {
            perform_hid_rsd(host, port, expected_device_id, rsd_action)
        })
        .await
        {
            Ok(Ok(())) => {
                return Ok(StatusCode::NO_CONTENT);
            }
            Ok(Err(rust_ios_device_core::DeviceError::DeviceIdMismatch)) => {}
            Ok(Err(rust_ios_device_core::DeviceError::ActionOutcomeUnknown)) => {
                return Err(StatusCode::SERVICE_UNAVAILABLE);
            }
            Ok(Err(error)) => {
                tracing::warn!(error = %error, io_kind = ?error.io_kind(), "RSD HID action failed; checking USB");
                rsd_unavailable = true;
            }
            Err(_) => {
                // A worker panic does not prove the gesture was never sent.
                return Err(StatusCode::SERVICE_UNAVAILABLE);
            }
        }
    }

    let devices = list_devices().await.map_err(|error| {
        tracing::warn!(error = %error, io_kind = ?error.io_kind(), "USB enumeration failed for HID action");
        StatusCode::SERVICE_UNAVAILABLE
    })?;
    let Some(device) = devices
        .into_iter()
        .find(|device| device.device_id == device_id)
    else {
        return Err(if rsd_unavailable {
            StatusCode::SERVICE_UNAVAILABLE
        } else {
            StatusCode::NOT_FOUND
        });
    };
    let udid = device.udid.unwrap_or_else(|| device.device_id.clone());
    tokio::task::spawn_blocking(move || perform_hid_usb(udid, action))
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .map_err(|error| {
            tracing::warn!(error = %error, io_kind = ?error.io_kind(), "USB HID action failed");
            StatusCode::SERVICE_UNAVAILABLE
        })?;
    Ok(StatusCode::NO_CONTENT)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let args = Args::parse();
    anyhow::ensure!(
        args.listen.ip().is_loopback(),
        "--listen must be a loopback address; use SSH forwarding for remote clients"
    );
    rust_ios_device_core::wifi::configure_transport(&args.transport).map_err(anyhow::Error::msg)?;
    if args.enable_wifi {
        rust_ios_device_core::wifi::enable(args.device.as_deref().expect("clap requires device"))
            .await?;
        println!(
            "{}",
            serde_json::json!({"outcome":"completed","code":"wifi_setting_enabled","network_verified":false})
        );
        return Ok(());
    }
    if args.wifi_direct {
        anyhow::ensure!(
            args.transport == "wifi",
            "--wifi-direct requires --transport wifi"
        );
        rust_ios_device_core::wireless::configure(
            args.device.clone().expect("clap requires device"),
            args.wifi_host,
        )
        .map_err(anyhow::Error::msg)?;
    }
    let device = args.device.map(Arc::new);
    anyhow::ensure!(
        args.rsd_host.is_some() == args.rsd_port.is_some(),
        "--rsd-host and --rsd-port must be supplied together"
    );
    if let Some(host) = args.rsd_host {
        anyhow::ensure!(
            is_allowed_rsd_ip(host),
            "RSD host must be a loopback, private, or link-local IP"
        );
    }
    let rsd = args.rsd_host.zip(args.rsd_port);
    if let Some(instance_id) = args.recover_usb_interface {
        anyhow::ensure!(
            !instance_id.is_empty() && instance_id.len() <= 1024,
            "invalid USB instance identifier"
        );
        let report = rust_ios_device_diagnostics::recover_usb_interface(
            device.as_deref().expect("clap requires device"),
            &instance_id,
            None,
            false,
        )
        .await;
        println!("{}", serde_json::to_string_pretty(&report)?);
        anyhow::ensure!(
            report.outcome == "completed",
            "USB recovery did not complete: {}",
            report.code
        );
        return Ok(());
    }
    let discovery = args
        .tunneld_url
        .as_deref()
        .map(|origin| {
            rsd_discovery::Discovery::new(
                origin,
                device.as_deref().expect("clap requires device").as_str(),
            )
        })
        .transpose()?
        .map(Arc::new);
    if args.doctor {
        let mut report = rust_ios_device_diagnostics::diagnose(
            rsd,
            device.as_deref().map(String::as_str),
            false,
        )
        .await;
        if let Some(discovery) = &discovery {
            discovery.augment_diagnostics(&mut report).await;
        }
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    if args.recover_service {
        let mut report =
            rust_ios_device_diagnostics::recover(rsd, device.as_deref().map(String::as_str)).await;
        if let Some(discovery) = &discovery {
            discovery.augment_diagnostics(&mut report.diagnostics).await;
        }
        println!("{}", serde_json::to_string_pretty(&report)?);
        anyhow::ensure!(
            report.outcome == "completed",
            "Service recovery did not complete: {}",
            report.code
        );
        return Ok(());
    }
    let token = args
        .token
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("RUST_IOS_DEVICE_TOKEN is required"))?;
    let runner_device = device.clone();
    let state = AppState {
        screen_gate: Arc::new(tokio::sync::Semaphore::new(1)),
        video_control: Arc::new(tokio::sync::Mutex::new(None)),
        rsd_discovery: discovery,
        transport_history: Arc::new(tokio::sync::Mutex::new(Default::default())),
        runner_bundle: args.wda_runner.clone().map(Arc::new),
        wda_tunnel_binding_reviewed: args.wda_tunnel_binding_reviewed,
        runner: Arc::new(tokio::sync::Mutex::new(None)),
        runner_failure: Arc::new(tokio::sync::Mutex::new(None)),
        wda_port: args.wda_port,
        require_wda: args.require_wda,
        elements: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        token: Arc::new(token),
        device,
        rsd,
        gate: Arc::new(tokio::sync::Semaphore::new(1)),
        allow_service_restart: args.allow_service_restart,
        allow_usb_interface_restart: args.allow_usb_interface_restart,
        last_recovery: Arc::new(tokio::sync::Mutex::new(None)),
    };
    let runner_state = state.clone();
    tokio::spawn(transport_monitor::run(state.clone()));
    let app = Router::new()
        .route("/v1/health", get(health))
        .route("/v1/devices", get(devices))
        .route("/v1/diagnostics", get(control::diagnostics))
        .route(
            "/v1/recovery/usb-interface",
            axum::routing::post(control::recover_usb),
        )
        .route("/v1/capabilities", get(control::capabilities))
        .route("/v1/devices/{device_id}/ui", get(elements::snapshot))
        .route(
            "/v1/recovery/elements",
            axum::routing::post(control::recover_elements),
        )
        .route(
            "/v1/devices/{device_id}/ui/find",
            axum::routing::post(elements::find),
        )
        .route(
            "/v1/devices/{device_id}/ui/activate",
            axum::routing::post(elements::activate),
        )
        .route(
            "/v1/recovery/service",
            axum::routing::post(control::recover),
        )
        .route("/v1/devices/{device_id}/screenshot.png", get(screenshot))
        .route("/v1/devices/{device_id}/screen.mjpeg", get(screen::stream))
        .route("/v1/devices/{device_id}/screen.video", get(video::stream))
        .route(
            "/v1/devices/{device_id}/touch",
            axum::routing::post(video::touch),
        )
        .route(
            "/v1/devices/{device_id}/apps",
            axum::routing::post(launch_app),
        )
        .route(
            "/v1/devices/{device_id}/actions",
            axum::routing::post(hid_action),
        )
        .with_state(state)
        .layer(axum::middleware::from_fn(control::envelope));
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    if let Some(bundle) = args.wda_runner {
        tracing::info!("starting configured XCTest runner");
        match rust_ios_device_core::elements::start_runner_with_binding_review(
            runner_device.as_deref().expect("clap requires device"),
            &bundle,
            args.wda_port,
            args.wda_tunnel_binding_reviewed,
        )
        .await
        {
            Ok(runner) => *runner_state.runner.lock().await = Some(runner),
            Err(error) => {
                let (stage, kind) = rust_ios_device_core::elements::startup_failure(&error);
                *runner_state.runner_failure.lock().await = Some((stage, kind));
                tracing::warn!(
                    stage,
                    kind,
                    "runner startup failed; daemon diagnostics and explicit recovery remain available"
                )
            }
        }
    }
    tracing::info!(address = %listener.local_addr()?, "daemon listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
