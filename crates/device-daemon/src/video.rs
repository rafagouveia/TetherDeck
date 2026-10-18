//! Authenticated HEVC streaming and controls sharing one CoreDevice session.
use super::*;
use axum::body::Bytes;
use rust_ios_device_api::{TouchPhase, TouchRequest, TouchResponse};
use rust_ios_device_core::{DeviceError, video::Video};
use tokio::sync::{mpsc, oneshot};

#[derive(Clone)]
pub(crate) struct LiveControl {
    pub device_id: String,
    session: uuid::Uuid,
    sender: mpsc::Sender<VideoCommand>,
}

enum VideoCommand {
    Touch {
        request: TouchRequest,
        reply: oneshot::Sender<Result<TouchResponse, DeviceError>>,
    },
    Action {
        action: HidAction,
        reply: oneshot::Sender<Result<(), DeviceError>>,
    },
    Capture {
        reply: oneshot::Sender<Result<Vec<u8>, DeviceError>>,
    },
}

/// Interactive phases only use the existing media session. They cannot start
/// a transport, reopen the display service or replay a failed contact.
pub async fn touch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<TouchRequest>,
) -> Result<Json<TouchResponse>, StatusCode> {
    if !authorized(&headers, &state) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || uuid::Uuid::parse_str(&request.gesture_id).is_err()
        || request.sequence == 0
        || (request.phase == TouchPhase::Start && request.sequence != 1)
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    if state
        .device
        .as_deref()
        .is_some_and(|allowed| allowed.as_str() != id)
    {
        return Err(StatusCode::FORBIDDEN);
    }
    let active = state
        .video_control
        .lock()
        .await
        .clone()
        .filter(|active| active.device_id == id)
        .ok_or(StatusCode::CONFLICT)?;
    let _permit = state
        .gate
        .clone()
        .try_acquire_owned()
        .map_err(|_| StatusCode::LOCKED)?;
    // A slow element query must not delay contact phases beyond their lease.
    let mut elements = state.elements.try_lock().map_err(|_| StatusCode::LOCKED)?;
    if let Some(session) = elements.get_mut(&id) {
        session.invalidate();
    }
    drop(elements);
    let (reply, receive) = oneshot::channel();
    active
        .sender
        .try_send(VideoCommand::Touch { request, reply })
        .map_err(|error| match error {
            mpsc::error::TrySendError::Closed(_) => StatusCode::CONFLICT,
            mpsc::error::TrySendError::Full(_) => StatusCode::LOCKED,
        })?;
    match tokio::time::timeout(std::time::Duration::from_millis(850), receive).await {
        Ok(Ok(Ok(response))) => Ok(Json(response)),
        Ok(Ok(Err(error))) => {
            tracing::warn!(error = %error, "live touch phase failed; no replay");
            Err(match error {
                DeviceError::InvalidTouchSequence | DeviceError::TouchBusy => StatusCode::CONFLICT,
                DeviceError::InvalidTouchCoordinates => StatusCode::BAD_REQUEST,
                _ => StatusCode::SERVICE_UNAVAILABLE,
            })
        }
        Ok(Err(_)) => Err(StatusCode::SERVICE_UNAVAILABLE),
        Err(_) => Err(StatusCode::GATEWAY_TIMEOUT),
    }
}

/// None means there is no active media session for this exact scoped device.
/// A failed/ambiguous command is NEVER replayed through the old media handshake.
pub(crate) async fn action_if_active(
    state: &AppState,
    id: &str,
    action: HidAction,
) -> Option<Result<(), DeviceError>> {
    let active = state.video_control.lock().await.clone()?;
    if active.device_id != id {
        return None;
    }
    let (reply, receive) = oneshot::channel();
    if active
        .sender
        .try_send(VideoCommand::Action { action, reply })
        .is_err()
    {
        return Some(Err(DeviceError::RsdTransport(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "live control is busy or closed",
        ))));
    }
    Some(
        tokio::time::timeout(std::time::Duration::from_secs(66), receive)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(Err(DeviceError::ActionOutcomeUnknown)),
    )
}

pub(crate) async fn screenshot_if_active(
    state: &AppState,
    id: &str,
) -> Option<Result<Vec<u8>, DeviceError>> {
    let active = state.video_control.lock().await.clone()?;
    if active.device_id != id {
        return None;
    }
    let (reply, receive) = oneshot::channel();
    if active
        .sender
        .try_send(VideoCommand::Capture { reply })
        .is_err()
    {
        return Some(Err(DeviceError::RsdTransport(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "live control is busy or closed",
        ))));
    }
    Some(
        tokio::time::timeout(std::time::Duration::from_secs(8), receive)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(Err(DeviceError::Timeout)),
    )
}

struct Ready {
    width: u32,
    height: u32,
    geometry: Option<rust_ios_device_core::video::HevcGeometry>,
    frame: Vec<u8>,
}

struct StartupFailure {
    code: &'static str,
    message: String,
    retryable: bool,
}

impl StartupFailure {
    fn from_device(error: &DeviceError) -> Self {
        let code = match error {
            DeviceError::DeviceIdMismatch => "device_mismatch",
            DeviceError::WifiPairingRequired => "wifi_pairing_required",
            DeviceError::Idevice(idevice::IdeviceError::InvalidHostID) => "video_pairing_invalid",
            DeviceError::Idevice(idevice::IdeviceError::DeveloperModeNotEnabled) => {
                "video_developer_mode_required"
            }
            DeviceError::Idevice(
                idevice::IdeviceError::ImageNotMounted | idevice::IdeviceError::ServiceNotFound,
            ) => "video_developer_services_required",
            _ => "screen_video_unavailable",
        };
        Self { code, message: "Não foi possível iniciar o vídeo HEVC do iPhone. Confira pareamento remoto, Modo de Desenvolvedor e aparelho desbloqueado.".into(), retryable: code == "screen_video_unavailable" }
    }
}

/// Wire v1: repeated 24-byte headers followed by a complete Annex-B HEVC AU.
/// All integers little endian: payload_len:u32, rtp_timestamp:u32, sequence:u32,
/// flags:u32 (bit 0 IRAP), host_elapsed_us:u64. HTTP chunk boundaries are arbitrary.
pub async fn stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, StatusCode> {
    if !authorized(&headers, &state) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if id.is_empty()
        || id.len() > 128
        || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    if state
        .device
        .as_deref()
        .is_some_and(|allowed| allowed.as_str() != id)
    {
        return Err(StatusCode::FORBIDDEN);
    }
    let permit = state
        .screen_gate
        .clone()
        .try_acquire_owned()
        .map_err(|_| StatusCode::LOCKED)?;
    let target = state.rsd_target().await;
    if state.has_rsd() && target.is_none() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let rsd = target.map(|target| {
        (
            target.host,
            target.port,
            target.expected_rsd_id.unwrap_or_else(|| id.clone()),
        )
    });
    let (ready_tx, ready_rx) = oneshot::channel::<Result<Ready, StartupFailure>>();
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(3);
    let session = uuid::Uuid::new_v4();
    let shared_control = state.video_control.clone();
    std::thread::Builder::new().name("tetherdeck-video".into()).stack_size(16 * 1024 * 1024).spawn(move || {
        let _permit = permit;
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return; };
        tokio::task::LocalSet::new().block_on(&runtime, async move {
            let mut video = match Video::connect(id.clone(), rsd).await {
                Ok(video) => video,
                Err(error) => {
                    tracing::warn!(error=%error, io_kind=?error.io_kind(), "encoded display stream startup failed");
                    let _ = ready_tx.send(Err(StartupFailure::from_device(&error)));
                    return;
                }
            };
            let first = tokio::select! {
                _ = tx.closed() => { video.close().await; return; }
                result = tokio::time::timeout(std::time::Duration::from_secs(15), video.frame()) => result,
            };
            let first = match first {
                Ok(Ok(frame)) => frame,
                error => {
                    tracing::warn!(?error, "encoded display stream did not produce a keyframe");
                    let _ = ready_tx.send(Err(StartupFailure { code: "screen_video_unavailable", message: "O iPhone abriu o serviço, mas não enviou um quadro HEVC completo. Consulte o diagnóstico de vídeo.".into(), retryable: true }));
                    video.close().await;
                    return;
                }
            };
            let Some(mut control) = video.take_control() else { video.close().await; return; };
            let (command_tx, mut command_rx) = mpsc::channel::<VideoCommand>(1);
            {
                let mut active = shared_control.lock().await;
                *active = Some(LiveControl { device_id: id, session, sender: command_tx });
            }
            let (stop_commands, mut stopping) = tokio::sync::watch::channel(false);
            let mut commands = tokio::task::spawn_local(async move {
                let mut touch_lease = tokio::time::interval(std::time::Duration::from_millis(100));
                touch_lease.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    if *stopping.borrow() { break; }
                    let command = tokio::select! {
                        biased;
                        _ = stopping.changed() => break,
                        _ = touch_lease.tick() => {
                            if control.touch_active() {
                                if let Err(error) = control.expire_touch().await {
                                    tracing::warn!(error = %error, "live touch lease cleanup failed");
                                }
                            }
                            continue;
                        }
                        command = command_rx.recv() => match command { Some(command) => command, None => break },
                    };
                    // Once a mutation has begun, finish its release/end sequence.
                    // Closing the viewer cannot cancel between Down and Up.
                    match command {
                        VideoCommand::Touch { request, reply } => {
                            // A timed-out HTTP caller cannot leave an unstarted
                            // contact queued behind a long legacy action.
                            if reply.is_closed()
                                && !matches!(request.phase, TouchPhase::End | TouchPhase::Cancel)
                            { continue; }
                            let result = control.touch(request).await;
                            let _ = reply.send(result);
                        }
                        VideoCommand::Action { action, reply } => {
                            let result = control.action(action).await;
                            let _ = reply.send(result);
                        }
                        VideoCommand::Capture { reply } => {
                            // Capture can wait six seconds; never block the
                            // 750ms release lease behind that independent read.
                            if control.touch_active() {
                                let _ = reply.send(Err(DeviceError::TouchBusy));
                                continue;
                            }
                            let result = control.screenshot_png().await;
                            let _ = reply.send(result);
                        }
                    }
                }
                if control.touch_active() {
                    if let Err(error) = control.cancel_touch().await {
                        tracing::warn!(error = %error, "live touch shutdown release failed");
                    }
                }
            });
            let ready = Ready { width: video.source_width, height: video.source_height, geometry: video.geometry, frame: packet(first) };
            if ready_tx.send(Ok(ready)).is_ok() {
                loop {
                    let frame = tokio::select! {
                        _ = tx.closed() => break,
                        result = video.frame() => match result {
                            Ok(frame) => frame,
                            Err(error) => { tracing::warn!(error=%error, "encoded display stream stopped"); break; }
                        },
                    };
                    // Do not accumulate seconds of stale screen data. If one AU
                    // cannot fit, suppress dependent pictures until a fresh IRAP
                    // with parameter sets arrives. The decoder sees only complete
                    // independent recovery points, never arbitrary missing P-frames.
                    match tx.try_send(packet(frame)) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Closed(_)) => break,
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            tracing::warn!("encoded video consumer lagged; requesting an independent frame");
                            video.request_resync();
                        }
                    }
                }
            }
            {
                let mut active = shared_control.lock().await;
                if active.as_ref().is_some_and(|control| control.session == session) { *active = None; }
            }
            let _ = stop_commands.send(true);
            // Control may need 65 seconds of execution plus 3 seconds of
            // release/reset/remove cleanup after a timeout. HTTP may already
            // have reported unknown, but viewer closure must still await it.
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(70);
            let mut drain_media = true;
            loop {
                tokio::select! {
                    _ = &mut commands => break,
                    _ = tokio::time::sleep_until(deadline) => {
                        tracing::warn!("live control did not finish during shutdown; outcome is unknown");
                        commands.abort();
                        break;
                    }
                    result = video.frame(), if drain_media => {
                        // Keep encoder feedback flowing while a gesture finishes.
                        // Its frames are discarded after the HTTP consumer closes.
                        if result.is_err() { drain_media = false; }
                    }
                }
            }
            video.close().await;
        });
    }).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;

    let ready = tokio::time::timeout(std::time::Duration::from_secs(42), ready_rx)
        .await
        .map_err(|_| StatusCode::GATEWAY_TIMEOUT)?
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let ready = match ready {
        Ok(ready) => ready,
        Err(failure) => {
            let problem = rust_ios_device_api::ApiProblem {
                code: failure.code.into(), message: failure.message, request_id: String::new(),
                retryable: failure.retryable, execution_state: "not_started".into(),
                remediation: vec!["Consultar diagnóstico de vídeo. A transmissão não muda automaticamente de Wi-Fi para USB.".into()],
            };
            let mut response = Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&problem).unwrap()))
                .unwrap();
            response.extensions_mut().insert(problem);
            return Ok(response);
        }
    };
    let body = async_stream::try_stream! {
        yield Bytes::from(ready.frame);
        while let Some(frame) = rx.recv().await { yield Bytes::from(frame); }
    };
    let body: std::pin::Pin<
        Box<dyn futures_core::Stream<Item = Result<Bytes, std::io::Error>> + Send>,
    > = Box::pin(body);
    let mut response = Response::builder()
        .header(header::CONTENT_TYPE, "application/vnd.tetherdeck.hevc")
        .header(header::CACHE_CONTROL, "no-store")
        .header("x-content-type-options", "nosniff")
        .header("x-tetherdeck-video", "1")
        .header("x-source-width", ready.width.to_string())
        .header("x-source-height", ready.height.to_string());
    if let Some(geometry) = ready.geometry {
        response = response
            .header("x-hevc-coded-width", geometry.coded_width.to_string())
            .header("x-hevc-coded-height", geometry.coded_height.to_string())
            .header("x-hevc-display-width", geometry.display_width.to_string())
            .header("x-hevc-display-height", geometry.display_height.to_string())
            .header(
                "x-hevc-conformance-crop",
                format!(
                    "{},{},{},{}",
                    geometry.crop_left,
                    geometry.crop_top,
                    geometry.crop_right,
                    geometry.crop_bottom
                ),
            );
    }
    Ok(response.body(Body::from_stream(body)).unwrap())
}

fn packet(frame: rust_ios_device_core::video::VideoFrame) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(24 + frame.annex_b.len());
    bytes.extend_from_slice(&(frame.annex_b.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&frame.rtp_timestamp.to_le_bytes());
    bytes.extend_from_slice(&frame.sequence.to_le_bytes());
    bytes.extend_from_slice(&u32::from(frame.keyframe).to_le_bytes());
    bytes.extend_from_slice(&frame.elapsed_us.to_le_bytes());
    bytes.extend_from_slice(&frame.annex_b);
    bytes
}
