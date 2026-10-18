use crate::DeviceError;
use idevice::{
    ReadWrite,
    provider::RsdProvider,
    services::{
        core_device::{
            display_stream::{
                CallInfoBlob, DisplayServiceClient, build_screen_video_offer,
                build_start_video_parameters,
            },
            hid::{
                ButtonState, DigitizerEdge, DigitizerEventType, DigitizerTarget, IndigoHidClient, KeyboardUsage, MainKeyboardService,
                TOUCHSCREEN_STATE_CONTACT, TOUCHSCREEN_STATE_RELEASE, UniversalHidServiceClient,
            },
        },
        rsd::RsdHandshake,
    },
};
use rust_ios_device_api::{DeviceButton, HidAction, TouchPhase, TouchRequest, TouchResponse};
use std::{
    net::{IpAddr, SocketAddr},
    time::{Duration, Instant},
};

pub fn perform_hid_rsd(
    addr: IpAddr,
    port: u16,
    expected_device_id: String,
    action: HidAction,
) -> Result<(), DeviceError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(DeviceError::RsdTransport)?;
    runtime.block_on(async move {
        tokio::time::timeout(Duration::from_secs(90), async {
            let stream = tokio::net::TcpStream::connect((addr, port)).await?;
            let mut handshake = RsdHandshake::new(stream).await?;
            if handshake.uuid != expected_device_id {
                return Err(DeviceError::DeviceIdMismatch);
            }
            // Discover the routed local address, then reserve one UDP endpoint for
            // the short-lived audio/video stream required to authorize Indigo HID.
            let wildcard: IpAddr = if addr.is_ipv4() { "0.0.0.0" } else { "::" }
                .parse()
                .expect("constant IP address");
            let probe = tokio::net::UdpSocket::bind(SocketAddr::new(wildcard, 0)).await?;
            probe.connect(SocketAddr::new(addr, port)).await?;
            let local_ip = probe.local_addr()?.ip();
            let media_socket = tokio::net::UdpSocket::bind(SocketAddr::new(local_ip, 0)).await?;
            let media_addr = media_socket.local_addr()?;

            let mut provider = addr;
            let mut display = start_display_stream(
                &mut handshake,
                &mut provider,
                media_addr.ip(),
                media_addr.port(),
                addr,
            )
            .await?;

            tokio::time::sleep(Duration::from_millis(1200)).await;

            // Keep the negotiated stream alive while draining and discarding its
            // media packets; no video or audio content is retained or exposed.
            let media_receiver = tokio::spawn(async move {
                let mut packet = [0u8; 2048];
                while media_socket.recv(&mut packet).await.is_ok() {}
            });

            let action_result = run_action(&mut handshake, &mut provider, action).await;
            let stop_result = display.stop_media_stream().await;
            media_receiver.abort();
            action_result.map_err(|_| DeviceError::ActionOutcomeUnknown)?;
            stop_result.map_err(|_| DeviceError::ActionOutcomeUnknown)?;
            Ok(())
        })
        .await
        .map_err(|_| DeviceError::ActionOutcomeUnknown)?
    })
}

/// Performs HID over the authenticated USB CoreDeviceProxy tunnel, including
/// the UDP media channel required by the device's DisplayService.
pub fn perform_hid_usb(udid: String, action: HidAction) -> Result<(), DeviceError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(DeviceError::RsdTransport)?;
    runtime.block_on(async move {
        tokio::time::timeout(Duration::from_secs(90), async {
            let (mut tunnel, mut handshake) = super::usb_rsd_stack(udid).await?;
            let media = tunnel.bind_udp(0).await?;
            let media_ip = tunnel.host_ip();
            let media_port = media.local_port();
            let sender_ip = tunnel.peer_ip();
            let mut display =
                start_display_stream(&mut handshake, &mut tunnel, media_ip, media_port, sender_ip)
                    .await?;

            // Give the device time to open dtuhidd's synthetic input gate.
            tokio::time::sleep(Duration::from_millis(1200)).await;

            // Discard media packets while keeping the tunnel channel serviced.
            let media_receiver = tokio::spawn(async move { while media.recv().await.is_ok() {} });
            let action_result = run_action(&mut handshake, &mut tunnel, action).await;
            let stop_result = display.stop_media_stream().await;
            media_receiver.abort();
            action_result.map_err(|_| DeviceError::ActionOutcomeUnknown)?;
            stop_result.map_err(|_| DeviceError::ActionOutcomeUnknown)?;
            Ok(())
        })
        .await
        .map_err(|_| DeviceError::ActionOutcomeUnknown)?
    })
}

type DisplayClient = DisplayServiceClient<Box<dyn ReadWrite>>;

async fn start_display_stream(
    handshake: &mut RsdHandshake,
    provider: &mut impl RsdProvider,
    receiver_ip: IpAddr,
    media_port: u16,
    sender_ip: IpAddr,
) -> Result<DisplayClient, DeviceError> {
    let call_id = uuid::Uuid::new_v4();
    let call_info = CallInfoBlob {
        call_id: call_id.as_u128() as u32,
        client_version: 1,
        device_type: "Windows".into(),
        framework_version: format!("rust-ios-device/{}", env!("CARGO_PKG_VERSION")),
        os_version: std::env::consts::OS.into(),
        device_name: None,
        audio_device_uid: None,
    };
    let mut display: DisplayClient = handshake.connect(provider).await?;
    let video_offer = build_screen_video_offer(
        &call_id.to_string(),
        &call_info,
        uuid::Uuid::new_v4().as_u128() as u32,
    )?;
    let session_id = uuid::Uuid::new_v4();
    let receiver_ip = receiver_ip.to_string();
    let sender_ip = sender_ip.to_string();
    display
        .start_media_stream(build_start_video_parameters(
            &receiver_ip,
            media_port,
            &sender_ip,
            None,
            video_offer,
            140,
            1,
            session_id,
        ))
        .await?;
    Ok(display)
}

pub fn validate_hid_action(action: &HidAction) -> Result<(), DeviceError> {
    match action {
        HidAction::Tap { .. } | HidAction::DeviceButton { .. } => Ok(()),
        HidAction::Press { duration_ms, .. } | HidAction::Drag { duration_ms, .. } => {
            validate_duration(*duration_ms)
        }
        HidAction::TypeText { text } if text.len() > 4096 => Err(DeviceError::InvalidTextLength),
        HidAction::TypeText { text }
            if !text
                .chars()
                .all(|character| keyboard_usage(character).is_some() || latin_key(character).is_some()) =>
        {
            Err(DeviceError::UnsupportedText)
        }
        HidAction::TypeText { .. } => Ok(()),
        HidAction::KeyPress { usage, modifiers } if (0x04..=0x73).contains(usage) && *modifiers <= 0x0f => Ok(()),
        HidAction::KeyPress { .. } => Err(DeviceError::UnsupportedText),
    }
}

pub(crate) async fn run_action(
    handshake: &mut RsdHandshake,
    provider: &mut impl RsdProvider,
    action: HidAction,
) -> Result<(), DeviceError> {
    HidSession::default()
        .action(handshake, provider, action, None)
        .await
}

/// Keep the one-way HID channels alive for the lifetime of a live media session.
/// A successful send confirms transport submission, never physical execution.
#[derive(Default)]
pub(crate) struct HidSession {
    indigo: Option<IndigoHidClient<Box<dyn ReadWrite>>>,
    universal: Option<UniversalHidServiceClient<Box<dyn ReadWrite>>>,
    pending_button: Option<u64>,
    pending_touch: Option<(u16, u16)>,
    pending_edge: Option<(f64, f64)>,
    keyboard: Option<MainKeyboardService>,
    keyboard_creation_unknown: bool,
    submitted: bool,
    invalid: bool,
    continuous_touch: Option<ContinuousTouch>,
    last_ended_touch: Option<(String, u64)>,
}

struct ContinuousTouch {
    gesture_id: String,
    sequence: u64,
    activity: Instant,
}

const TOUCH_LEASE: Duration = Duration::from_millis(750);
const TOUCH_SEND_BUDGET: Duration = Duration::from_millis(250);
const TOUCH_CLEANUP_BUDGET: Duration = Duration::from_millis(500);

impl HidSession {
    pub(crate) fn touch_active(&self) -> bool {
        self.continuous_touch.is_some()
    }

    pub(crate) async fn touch(
        &mut self,
        handshake: &mut RsdHandshake,
        provider: &mut impl RsdProvider,
        request: TouchRequest,
        source_size: [u32; 2],
    ) -> Result<TouchResponse, DeviceError> {
        if self.invalid {
            return Err(DeviceError::ActionOutcomeUnknown);
        }
        if request.sequence == 0 || uuid::Uuid::parse_str(&request.gesture_id).is_err() {
            return Err(DeviceError::InvalidTouchSequence);
        }
        if request.phase == TouchPhase::Start && request.sequence != 1 {
            return Err(DeviceError::InvalidTouchSequence);
        }
        if !source_size.iter().all(|n| (1..=8192).contains(n))
            || u32::from(request.x) >= source_size[0]
            || u32::from(request.y) >= source_size[1]
        {
            return Err(DeviceError::InvalidTouchCoordinates);
        }
        // A delayed sample must never revive a gesture whose lease expired.
        if self
            .continuous_touch
            .as_ref()
            .is_some_and(|touch| touch.activity.elapsed() >= TOUCH_LEASE)
        {
            self.cancel_touch(handshake, provider).await?;
            return Err(DeviceError::InvalidTouchSequence);
        }
        let releasing = matches!(request.phase, TouchPhase::End | TouchPhase::Cancel);
        if releasing && self.continuous_touch.is_none() {
            if self
                .last_ended_touch
                .as_ref()
                .is_some_and(|(id, seq)| id == &request.gesture_id && request.sequence >= *seq)
            {
                return Ok(TouchResponse {
                    gesture_id: request.gesture_id,
                    sequence: request.sequence,
                    active: false,
                });
            }
            return Err(DeviceError::InvalidTouchSequence);
        }
        match request.phase {
            TouchPhase::Start => {
                if self.continuous_touch.is_some() {
                    return Err(DeviceError::TouchBusy);
                }
                if self
                    .last_ended_touch
                    .as_ref()
                    .is_some_and(|(id, _)| id == &request.gesture_id)
                {
                    return Err(DeviceError::InvalidTouchSequence);
                }
                self.continuous_touch = Some(ContinuousTouch {
                    gesture_id: request.gesture_id.clone(),
                    sequence: request.sequence,
                    activity: Instant::now(),
                });
            }
            _ => {
                let touch = self
                    .continuous_touch
                    .as_mut()
                    .ok_or(DeviceError::InvalidTouchSequence)?;
                if touch.gesture_id != request.gesture_id || request.sequence <= touch.sequence {
                    return Err(DeviceError::InvalidTouchSequence);
                }
                touch.sequence = request.sequence;
                touch.activity = Instant::now();
            }
        }
        // System gestures need Indigo's explicit edge identity. Ordinary
        // touchscreen reports are intentionally retained for application input.
        // Indigo uses unit ratios, not physical pixels or UniversalHID's u16.
        // Protocol evidence: facebook/idb SimulatorDTUHIDTransport.swift and
        // ipbtools/ipb docs/protocol.md (HID Digitizer); no implementation copied.
        let edge_point = (f64::from(request.x) / f64::from(source_size[0]), f64::from(request.y) / f64::from(source_size[1]));
        if request.phase == TouchPhase::Start && u32::from(request.y) >= source_size[1].saturating_sub((source_size[1] / 50).max(1)) {
            self.pending_edge = Some(edge_point);
        }
        if self.pending_edge.is_some() {
            if request.phase != TouchPhase::Cancel { self.pending_edge = Some(edge_point); }
            if releasing {
                self.cancel_touch(handshake, provider).await?;
            } else {
                let event = if request.phase == TouchPhase::Start { DigitizerEventType::Start } else { DigitizerEventType::Position };
                if self.send_edge_sample(handshake, provider, event).await.is_err() {
                    // Only termination is allowed after an uncertain submission.
                    let _ = self.cancel_touch(handshake, provider).await;
                    self.invalid = true;
                    return Err(DeviceError::ActionOutcomeUnknown);
                }
                if let Some(touch) = self.continuous_touch.as_mut() { touch.activity = Instant::now(); }
            }
            return Ok(TouchResponse { gesture_id: request.gesture_id, sequence: request.sequence, active: !releasing });
        }
        let point = normalize_touch((f64::from(request.x), f64::from(request.y)), source_size);
        if request.phase != TouchPhase::Cancel {
            self.pending_touch = Some(point);
        }
        if releasing {
            self.cancel_touch(handshake, provider).await?;
        } else {
            let sent = self
                .send_touch_sample(handshake, provider, TOUCHSCREEN_STATE_CONTACT, point)
                .await;
            if sent.is_err() {
                // Only RELEASE may be retried on a fresh channel; never replay a
                // timed-out Start/Move. The whole gesture is now closed.
                let _ = self.cleanup_continuous_touch(handshake, provider).await;
                return Err(DeviceError::ActionOutcomeUnknown);
            }
            if let Some(touch) = self.continuous_touch.as_mut() {
                touch.activity = Instant::now();
            }
        }
        Ok(TouchResponse {
            gesture_id: request.gesture_id,
            sequence: request.sequence,
            active: !releasing,
        })
    }

    async fn send_touch_sample(
        &mut self,
        handshake: &mut RsdHandshake,
        provider: &mut impl RsdProvider,
        state: u8,
        point: (u16, u16),
    ) -> Result<(), DeviceError> {
        tokio::time::timeout(TOUCH_SEND_BUDGET, async {
            if self.universal.is_none() {
                self.universal = Some(handshake.connect(provider).await?);
            }
            self.universal
                .as_mut()
                .expect("initialized HID channel")
                .send_touchscreen(state, point.0, point.1, None)
                .await?;
            Ok::<(), DeviceError>(())
        })
        .await
        .map_err(|_| DeviceError::ActionOutcomeUnknown)?
    }

    async fn send_edge_sample(&mut self, handshake: &mut RsdHandshake, provider: &mut impl RsdProvider, event: DigitizerEventType) -> Result<(), DeviceError> {
        let point = self.pending_edge.ok_or(DeviceError::InvalidTouchSequence)?;
        tokio::time::timeout(TOUCH_SEND_BUDGET, async {
            if self.indigo.is_none() { self.indigo = Some(handshake.connect(provider).await?); }
            self.indigo.as_mut().expect("initialized Indigo channel").send_digitizer(point, None, event, DigitizerEdge::Bottom, DigitizerTarget::MainScreen).await?;
            Ok::<(),DeviceError>(())
        }).await.map_err(|_| DeviceError::ActionOutcomeUnknown)?
    }

    fn end_touch_record(&mut self) {
        if let Some(touch) = self.continuous_touch.take() {
            self.last_ended_touch = Some((touch.gesture_id, touch.sequence));
        }
    }

    async fn cleanup_continuous_touch(
        &mut self,
        handshake: &mut RsdHandshake,
        provider: &mut impl RsdProvider,
    ) -> Result<(), DeviceError> {
        // The prior stream may contain a partially written XPC message. A new
        // channel on the same provider is used only for this contact's release.
        self.universal = None;
        let point = self.pending_touch;
        let released = tokio::time::timeout(TOUCH_CLEANUP_BUDGET, async {
            if let Some((x, y)) = point {
                let mut hid: UniversalHidServiceClient<Box<dyn ReadWrite>> =
                    handshake.connect(provider).await?;
                hid.send_touchscreen(TOUCHSCREEN_STATE_RELEASE, x, y, None)
                    .await?;
                self.universal = Some(hid);
            }
            Ok::<(), DeviceError>(())
        })
        .await;
        self.end_touch_record();
        if matches!(released, Ok(Ok(()))) {
            self.pending_touch = None;
            Ok(())
        } else {
            self.invalid = true;
            self.universal = None;
            Err(DeviceError::ActionOutcomeUnknown)
        }
    }

    pub(crate) async fn expire_touch(
        &mut self,
        handshake: &mut RsdHandshake,
        provider: &mut impl RsdProvider,
    ) -> Result<(), DeviceError> {
        if self
            .continuous_touch
            .as_ref()
            .is_some_and(|touch| touch.activity.elapsed() >= TOUCH_LEASE)
        {
            self.cancel_touch(handshake, provider).await?;
        }
        Ok(())
    }

    pub(crate) async fn cancel_touch(
        &mut self,
        handshake: &mut RsdHandshake,
        provider: &mut impl RsdProvider,
    ) -> Result<(), DeviceError> {
        if self.pending_edge.is_some() {
            let mut released = self.send_edge_sample(handshake, provider, DigitizerEventType::End).await.is_ok();
            if !released {
                self.indigo = None;
                released = self.send_edge_sample(handshake, provider, DigitizerEventType::End).await.is_ok();
            }
            self.end_touch_record();
            if released { self.pending_edge = None; return Ok(()) }
            self.invalid = true; self.indigo = None;
            return Err(DeviceError::ActionOutcomeUnknown);
        }
        let Some(point) = self.pending_touch else {
            self.end_touch_record();
            return Ok(());
        };
        if self
            .send_touch_sample(handshake, provider, TOUCHSCREEN_STATE_RELEASE, point)
            .await
            .is_ok()
        {
            self.pending_touch = None;
            self.end_touch_record();
            Ok(())
        } else {
            self.cleanup_continuous_touch(handshake, provider).await
        }
    }

    pub(crate) async fn action(
        &mut self,
        handshake: &mut RsdHandshake,
        provider: &mut impl RsdProvider,
        action: HidAction,
        source_size: Option<[u32; 2]>,
    ) -> Result<(), DeviceError> {
        if self.invalid {
            return Err(DeviceError::ActionOutcomeUnknown);
        }
        if self.touch_active() {
            return Err(DeviceError::TouchBusy);
        }
        validate_hid_action(&action)?;
        self.submitted = false;
        let result = tokio::time::timeout(
            Duration::from_secs(65),
            self.action_inner(handshake, provider, action, source_size),
        )
        .await;
        match result {
            Ok(Ok(())) => {
                self.submitted = false;
                Ok(())
            }
            Ok(Err(error)) if !self.submitted => Err(error),
            _ => {
                // Keep cleanup outside the execution timeout. Only release/reset/
                // removal of this command is allowed; never replay a mutation.
                self.invalid = true;
                let cleanup =
                    tokio::time::timeout(Duration::from_secs(3), self.cleanup(handshake, provider))
                        .await;
                tracing::warn!(
                    cleanup_completed = matches!(cleanup, Ok(Ok(()))),
                    "HID command outcome unknown; session invalidated after bounded cleanup"
                );
                self.indigo = None;
                self.universal = None;
                self.keyboard = None;
                Err(DeviceError::ActionOutcomeUnknown)
            }
        }
    }

    async fn cleanup(
        &mut self,
        handshake: &mut RsdHandshake,
        provider: &mut impl RsdProvider,
    ) -> Result<(), DeviceError> {
        // A canceled write may have left an incomplete XPC frame. New service
        // channels share the same authenticated provider/media session, and are
        // used solely to release the pending state whose identity we own.
        self.indigo = None;
        self.universal = None;
        // Creation can reach the device before its reply provides the confirmed
        // owner token. Closing channels is safe; guessing an ID to remove is not.
        let mut failed = self.keyboard_creation_unknown;
        if let Some(usage) = self.pending_button {
            let mut hid: IndigoHidClient<Box<dyn ReadWrite>> = handshake.connect(provider).await?;
            if hid.send_button(0x0C, usage, ButtonState::Up).await.is_ok() {
                self.pending_button = None;
            } else {
                failed = true;
            }
        }
        if self.pending_touch.is_some() || self.keyboard.is_some() {
            let mut hid: UniversalHidServiceClient<Box<dyn ReadWrite>> =
                handshake.connect(provider).await?;
            if let Some((x, y)) = self.pending_touch {
                if hid
                    .send_touchscreen(TOUCHSCREEN_STATE_RELEASE, x, y, None)
                    .await
                    .is_ok()
                {
                    self.pending_touch = None;
                } else {
                    failed = true;
                }
            }
            if let Some(service) = self.keyboard.as_mut() {
                // remove_main_keyboard always attempts reset, removal, and a
                // confirmed absence. Never remove an unconfirmed service ID.
                if hid.remove_main_keyboard(service).await.is_ok() {
                    self.keyboard = None;
                } else {
                    failed = true;
                }
            }
        }
        if failed {
            Err(DeviceError::ActionOutcomeUnknown)
        } else {
            Ok(())
        }
    }

    async fn action_inner(
        &mut self,
        handshake: &mut RsdHandshake,
        provider: &mut impl RsdProvider,
        action: HidAction,
        source_size: Option<[u32; 2]>,
    ) -> Result<(), DeviceError> {
        let source_size = if matches!(
            &action,
            HidAction::Tap { .. } | HidAction::Press { .. } | HidAction::Drag { .. }
        ) {
            match source_size {
                Some(size) if size.iter().all(|n| (1..=8192).contains(n)) => Some(size),
                Some(_) => return Err(DeviceError::LiveFrameUnavailable),
                None => Some(measure_source_size(handshake, provider).await?),
            }
        } else {
            None
        };
        match action {
            HidAction::DeviceButton { button } => {
                if self.indigo.is_none() {
                    self.indigo = Some(handshake.connect(provider).await?);
                }
                let hid = self.indigo.as_mut().expect("initialized HID channel");
                // Consumer-page values are standard HID usages (Apple's
                // IOHIDUsageTables.h). iOS Indigo's Home/Lock interpretation and
                // minimum hold times are independently documented in the upstream
                // pymobiledevice3 remote/core_device/screen_stream.py button table:
                // https://github.com/doronz88/pymobiledevice3/blob/master/pymobiledevice3/remote/core_device/screen_stream.py#L96-L114
                // No implementation from that project is linked or copied here.
                let (usage, hold_ms) = match button {
                    DeviceButton::Home | DeviceButton::AppSwitcher => (0x40, 50),
                    DeviceButton::VolumeUp => (0xE9, 50),
                    DeviceButton::VolumeDown => (0xEA, 50),
                    DeviceButton::Lock => (0x30, 500),
                };
                self.submitted = true;
                press_device_button(hid, usage, hold_ms, &mut self.pending_button).await?;
                if button == DeviceButton::AppSwitcher {
                    // This second press is part of the requested double-Home
                    // gesture. Stop immediately if the first press was uncertain.
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    press_device_button(hid, usage, hold_ms, &mut self.pending_button).await?;
                }
            }
            HidAction::Tap { x, y } => {
                if self.universal.is_none() {
                    self.universal = Some(handshake.connect(provider).await?);
                }
                let hid = self.universal.as_mut().expect("initialized HID channel");
                self.submitted = true;
                gesture(
                    hid,
                    (f64::from(x), f64::from(y)),
                    Vec::new(),
                    50,
                    source_size.unwrap(),
                    &mut self.pending_touch,
                )
                .await?;
            }
            HidAction::Press { x, y, duration_ms } => {
                validate_duration(duration_ms)?;
                if self.universal.is_none() {
                    self.universal = Some(handshake.connect(provider).await?);
                }
                let hid = self.universal.as_mut().expect("initialized HID channel");
                self.submitted = true;
                gesture(
                    hid,
                    (f64::from(x), f64::from(y)),
                    Vec::new(),
                    duration_ms,
                    source_size.unwrap(),
                    &mut self.pending_touch,
                )
                .await?;
            }
            HidAction::Drag {
                start_x,
                start_y,
                end_x,
                end_y,
                duration_ms,
            } => {
                validate_duration(duration_ms)?;
                if self.universal.is_none() {
                    self.universal = Some(handshake.connect(provider).await?);
                }
                let hid = self.universal.as_mut().expect("initialized HID channel");
                self.submitted = true;
                let steps = (duration_ms / 16).clamp(1, 120);
                let mut positions = Vec::with_capacity(steps as usize);
                for step in 1..=steps {
                    let fraction = step as f64 / steps as f64;
                    let x = start_x as f64 + (end_x as f64 - start_x as f64) * fraction;
                    let y = start_y as f64 + (end_y as f64 - start_y as f64) * fraction;
                    positions.push((x, y));
                }
                gesture(
                    hid,
                    (f64::from(start_x), f64::from(start_y)),
                    positions,
                    duration_ms / steps,
                    source_size.unwrap(),
                    &mut self.pending_touch,
                )
                .await?;
            }
            action @ (HidAction::TypeText { .. } | HidAction::KeyPress { .. }) => {
                let (text, key) = match action {
                    HidAction::TypeText { text } => (text, None),
                    HidAction::KeyPress { usage, modifiers } => (String::new(), Some((usage, modifiers))),
                    _ => unreachable!(),
                };
                if text.len() > 4096 {
                    return Err(DeviceError::InvalidTextLength);
                }
                if self.universal.is_none() {
                    self.universal = Some(handshake.connect(provider).await?);
                }
                let keyboard = self.universal.as_mut().expect("initialized HID channel");
                // Store ownership outside the cancelable future so timeout
                // cleanup retains the confirmed keyboard and its pressed state.
                self.submitted = true;
                self.keyboard_creation_unknown = true;
                self.keyboard = Some(keyboard.create_main_keyboard().await?);
                self.keyboard_creation_unknown = false;
                let service = self.keyboard.as_mut().expect("confirmed keyboard");
                let result = if let Some((usage, modifiers)) = key {
                    press_key(keyboard, service, usage, modifiers).await
                } else { type_text(keyboard, service, &text).await };
                let cleanup = keyboard.remove_main_keyboard(service).await;
                if cleanup.is_ok() {
                    self.keyboard = None;
                }
                result?;
                cleanup?;
            }
        }
        Ok(())
    }
}

async fn measure_source_size(
    handshake: &mut RsdHandshake,
    provider: &mut impl RsdProvider,
) -> Result<[u32; 2], DeviceError> {
    use idevice::services::core_device::{ImageFormat, ScreenCaptureServiceClient};
    let mut capture: ScreenCaptureServiceClient<Box<dyn ReadWrite>> =
        handshake.connect(provider).await?;
    let png = capture.take_screenshot(None, ImageFormat::Png).await?;
    if png.len() < 24 || !png.starts_with(b"\x89PNG\r\n\x1a\n") || &png[12..16] != b"IHDR" {
        return Err(DeviceError::LiveFrameUnavailable);
    }
    let size = [
        u32::from_be_bytes(png[16..20].try_into().unwrap()),
        u32::from_be_bytes(png[20..24].try_into().unwrap()),
    ];
    if !size.iter().all(|n| (1..=8192).contains(n)) {
        return Err(DeviceError::LiveFrameUnavailable);
    }
    Ok(size)
}

async fn press_device_button(
    hid: &mut IndigoHidClient<Box<dyn ReadWrite>>,
    usage: u64,
    hold_ms: u64,
    pending: &mut Option<u64>,
) -> Result<(), DeviceError> {
    *pending = Some(usage);
    let down = hid.send_button(0x0C, usage, ButtonState::Down).await;
    if down.is_ok() {
        tokio::time::sleep(Duration::from_millis(hold_ms)).await;
    }
    // Attempt a release even if the press write failed after a partial send.
    // This is cleanup of the same press, never another press or retry.
    let release = hid.send_button(0x0C, usage, ButtonState::Up).await;
    if release.is_ok() {
        *pending = None;
    }
    down?;
    release?;
    Ok(())
}

async fn gesture(
    hid: &mut UniversalHidServiceClient<Box<dyn ReadWrite>>,
    start: (f64, f64),
    positions: Vec<(f64, f64)>,
    delay_ms: u32,
    source_size: [u32; 2],
    pending: &mut Option<(u16, u16)>,
) -> Result<(), DeviceError> {
    // IndigoDigitizerEvent uses a different peer-event envelope and can be
    // silently canceled. Use the vendor's native mainTouchscreen report API.
    // Protocol facts only (no upstream implementation copied):
    // https://github.com/doronz88/pymobiledevice3/blob/master/pymobiledevice3/remote/core_device/hid_service.py
    // https://github.com/doronz88/pymobiledevice3/blob/master/docs/guides/cli-recipes.md#hid-input
    let normalized = |point| normalize_touch(point, source_size);
    let first = normalized(start);
    *pending = Some(first);
    let contact = hid
        .send_touchscreen(TOUCHSCREEN_STATE_CONTACT, first.0, first.1, None)
        .await;
    let mut current = start;
    let movement = async {
        contact?;
        if positions.is_empty() {
            tokio::time::sleep(Duration::from_millis(delay_ms.into())).await;
        } else {
            for position in positions {
                let point = normalized(position);
                // Keep the attempted position for release even on a partial send.
                current = position;
                *pending = Some(point);
                hid.send_touchscreen(TOUCHSCREEN_STATE_CONTACT, point.0, point.1, None)
                    .await?;
                tokio::time::sleep(Duration::from_millis(delay_ms.into())).await;
            }
        }
        Ok::<(), DeviceError>(())
    }
    .await;
    let point = normalized(current);
    let release = hid
        .send_touchscreen(TOUCHSCREEN_STATE_RELEASE, point.0, point.1, None)
        .await;
    if release.is_ok() {
        *pending = None;
    }
    movement?;
    release?;
    Ok(())
}

fn validate_duration(duration_ms: u32) -> Result<(), DeviceError> {
    if (1..=60_000).contains(&duration_ms) {
        Ok(())
    } else {
        Err(DeviceError::InvalidHidDuration)
    }
}

fn normalize_touch(point: (f64, f64), source_size: [u32; 2]) -> (u16, u16) {
    let axis = |value: f64, size: u32| {
        ((value / f64::from(size)).clamp(0.0, 1.0) * f64::from(u16::MAX)).round() as u16
    };
    (axis(point.0, source_size[0]), axis(point.1, source_size[1]))
}

async fn type_text(
    keyboard: &mut UniversalHidServiceClient<Box<dyn ReadWrite>>,
    service: &mut idevice::services::core_device::hid::MainKeyboardService,
    text: &str,
) -> Result<(), DeviceError> {
    for character in text.chars() {
        if let Some((usage, shifted)) = keyboard_usage(character) {
            press_key(keyboard, service, usage, if shifted {2} else {0}).await?;
        } else if let Some((accent, base, shifted)) = latin_key(character) {
            // Apple US hardware layout: Option accent dead key, then the base.
            press_key(keyboard, service, accent, 4 | if base.is_none() && shifted {2} else {0}).await?;
            if let Some(base) = base { press_key(keyboard, service, base, if shifted {2} else {0}).await?; }
        } else { return Err(DeviceError::UnsupportedText) }
        tokio::time::sleep(Duration::from_millis(12)).await;
    }
    Ok(())
}

async fn press_key(keyboard: &mut UniversalHidServiceClient<Box<dyn ReadWrite>>, service: &mut MainKeyboardService, usage: u16, modifiers: u8) -> Result<(), DeviceError> {
    for bit in 0..4 { if modifiers & (1 << bit) != 0 { keyboard.main_keyboard_key_down(service, KeyboardUsage::new(0xe0 + bit)?).await?; } }
    let key = KeyboardUsage::new(usage)?;
    keyboard.main_keyboard_key_down(service, key).await?;
    keyboard.main_keyboard_key_up(service, key).await?;
    for bit in (0..4).rev() { if modifiers & (1 << bit) != 0 { keyboard.main_keyboard_key_up(service, KeyboardUsage::new(0xe0 + bit)?).await?; } }
    Ok(())
}

fn latin_key(c: char) -> Option<(u16, Option<u16>, bool)> {
    let lower = c.to_lowercase().next()?;
    let shifted = c.is_uppercase();
    let (accent, base) = match lower {
        'á'=>('e',Some('a')), 'é'=>('e',Some('e')), 'í'=>('e',Some('i')), 'ó'=>('e',Some('o')), 'ú'=>('e',Some('u')),
        'à'=>('`',Some('a')), 'è'=>('`',Some('e')), 'ì'=>('`',Some('i')), 'ò'=>('`',Some('o')), 'ù'=>('`',Some('u')),
        'â'=>('i',Some('a')), 'ê'=>('i',Some('e')), 'î'=>('i',Some('i')), 'ô'=>('i',Some('o')), 'û'=>('i',Some('u')),
        'ä'=>('u',Some('a')), 'ë'=>('u',Some('e')), 'ï'=>('u',Some('i')), 'ö'=>('u',Some('o')), 'ü'=>('u',Some('u')),
        'ã'=>('n',Some('a')), 'õ'=>('n',Some('o')), 'ñ'=>('n',Some('n')), 'ç'=>('c',None),
        _=>return None,
    };
    Some((keyboard_usage(accent)?.0, base.and_then(|b|keyboard_usage(b).map(|v|v.0)), shifted))
}

fn keyboard_usage(character: char) -> Option<(u16, bool)> {
    let usage = match character {
        'a'..='z' => return Some((0x04 + character as u16 - 'a' as u16, false)),
        'A'..='Z' => return Some((0x04 + character as u16 - 'A' as u16, true)),
        '1'..='9' => return Some((0x1E + character as u16 - '1' as u16, false)),
        '0' => 0x27,
        ' ' => 0x2C,
        '\n' | '\r' => 0x28,
        '\t' => 0x2B,
        '!' => return Some((0x1E, true)),
        '@' => return Some((0x1F, true)),
        '#' => return Some((0x20, true)),
        '$' => return Some((0x21, true)),
        '%' => return Some((0x22, true)),
        '^' => return Some((0x23, true)),
        '&' => return Some((0x24, true)),
        '*' => return Some((0x25, true)),
        '(' => return Some((0x26, true)),
        ')' => return Some((0x27, true)),
        '-' => 0x2D,
        '_' => return Some((0x2D, true)),
        '=' => 0x2E,
        '+' => return Some((0x2E, true)),
        '[' => 0x2F,
        '{' => return Some((0x2F, true)),
        ']' => 0x30,
        '}' => return Some((0x30, true)),
        '\\' => 0x31,
        '|' => return Some((0x31, true)),
        ';' => 0x33,
        ':' => return Some((0x33, true)),
        '\'' => 0x34,
        '"' => return Some((0x34, true)),
        '`' => 0x35,
        '~' => return Some((0x35, true)),
        ',' => 0x36,
        '<' => return Some((0x36, true)),
        '.' => 0x37,
        '>' => return Some((0x37, true)),
        '/' => 0x38,
        '?' => return Some((0x38, true)),
        _ => return None,
    };
    Some((usage, false))
}

#[cfg(test)]
mod tests {
    use super::{keyboard_usage, validate_duration, validate_hid_action};
    use crate::DeviceError;
    use rust_ios_device_api::HidAction;

    #[test]
    fn keyboard_mapping_covers_us_ascii_and_rejects_non_ascii() {
        assert_eq!(keyboard_usage('a'), Some((0x04, false)));
        assert_eq!(keyboard_usage('Z'), Some((0x1D, true)));
        assert_eq!(keyboard_usage('0'), Some((0x27, false)));
        assert_eq!(keyboard_usage('?'), Some((0x38, true)));
        assert_eq!(keyboard_usage('\n'), Some((0x28, false)));
        assert_eq!(keyboard_usage('é'), None);
    }

    #[test]
    fn duration_limits_are_inclusive() {
        assert!(validate_duration(1).is_ok());
        assert!(validate_duration(60_000).is_ok());
        assert!(matches!(
            validate_duration(0),
            Err(DeviceError::InvalidHidDuration)
        ));
        assert!(matches!(
            validate_duration(60_001),
            Err(DeviceError::InvalidHidDuration)
        ));
    }

    #[test]
    fn hid_contract_validation_rejects_unsupported_text_and_long_text() {
        assert!(matches!(
            validate_hid_action(&HidAction::TypeText {
                text: "🙂".into()
            }),
            Err(DeviceError::UnsupportedText)
        ));
        assert!(matches!(
            validate_hid_action(&HidAction::TypeText {
                text: "x".repeat(4097)
            }),
            Err(DeviceError::InvalidTextLength)
        ));
        assert!(
            validate_hid_action(&HidAction::TypeText {
                text: "Hello, iPhone!".into()
            })
            .is_ok()
        );
    }
}
