//! Persistent, encoded CoreDevice display stream. No screenshots or image transcoding.
//!
//! RTP follows the selected CoreDevice transport. Only complete HEVC access
//! units leave this module; bounded reordering and keyframe recovery prevent a
//! missing packet from becoming an ever-growing latency queue.
use crate::{DeviceError, HidAction};
use idevice::{
    ReadWrite,
    services::{
        core_device::display_stream::{
            CallInfoBlob, DisplayServiceClient, HevcDepacketizer, ReportBlock, RtpPacket,
            SenderReport, build_frame_ack, build_keyframe_request, build_liveness,
            build_screen_audio_offer, build_screen_video_offer, build_start_audio_parameters,
            build_start_video_parameters, is_rtcp,
        },
        rsd::RsdHandshake,
    },
    tcp::handle::{AdapterHandle, UdpSocketHandle},
};
use rust_ios_device_api::{TouchRequest, TouchResponse};
use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
mod sps;
pub use sps::HevcGeometry;

pub const MAX_ACCESS_UNIT: usize = 8 * 1024 * 1024;
const REORDER_PACKETS: usize = 64;
const REORDER_WAIT: Duration = Duration::from_millis(12);
// Measured for this CoreDevice DisplayService protocol by the primary source's
// _build_rctl_packet (screen_stream.py). This is deliberately an explicit
// version/protocol assumption, not a claim about every iOS RTP media clock.
const RCTL_CLOCK_ASSUMPTION_HZ: u32 = 24_000;
type Display = DisplayServiceClient<Box<dyn ReadWrite>>;

enum Provider {
    Tunnel(AdapterHandle),
    Direct(IpAddr),
}

enum MediaSocket {
    Tunnel(UdpSocketHandle),
    Direct(tokio::net::UdpSocket, IpAddr),
}

impl MediaSocket {
    async fn receive(&self) -> Result<(Vec<u8>, u16), DeviceError> {
        match self {
            Self::Tunnel(socket) => {
                let datagram = socket.recv().await?;
                Ok((datagram.data, datagram.source_port))
            }
            Self::Direct(socket, expected) => {
                let mut packet = vec![0; 65_536];
                loop {
                    let (size, peer) = socket.recv_from(&mut packet).await?;
                    if peer.ip() == *expected {
                        packet.truncate(size);
                        return Ok((packet, peer.port()));
                    }
                }
            }
        }
    }

    async fn send(&self, port: u16, bytes: Vec<u8>) -> Result<(), DeviceError> {
        // RTCP replies use the authenticated tunnel's peer and the observed media
        // source port. A caller cannot redirect these reports to another host.
        tokio::time::timeout(Duration::from_millis(500), async {
            match self {
                Self::Tunnel(socket) => socket.send_to(port, bytes).await?,
                Self::Direct(socket, host) => {
                    socket.send_to(&bytes, SocketAddr::new(*host, port)).await?;
                }
            }
            Ok::<_, DeviceError>(())
        })
        .await
        .map_err(|_| DeviceError::Timeout)?
    }
}

struct AudioReceiver(tokio::task::JoinHandle<()>);

impl Drop for AudioReceiver {
    fn drop(&mut self) {
        // Also covers cancellation while video startup is still awaiting XPC.
        self.0.abort();
    }
}

#[derive(Clone, Copy)]
struct MediaFeedback {
    port: u16,
    our_ssrc: u32,
    remote_ssrc: u32,
    payload_type: u8,
}

impl MediaFeedback {
    fn negotiated(answer: &plist::Value) -> Option<Self> {
        let config = answer
            .as_dictionary()?
            .get("connection")?
            .as_dictionary()?
            .get("streamConfig")?
            .as_dictionary()?;
        let integer = |key| config.get(key)?.as_unsigned_integer();
        let port = u16::try_from(integer("SourcePort")?).ok()?;
        let our_ssrc = u32::try_from(integer("RemoteSSRC")?).ok()?;
        let remote_ssrc = u32::try_from(integer("LocalSSRC")?).ok()?;
        let payload_type = u8::try_from(integer("RxPayloadType")?).ok()?;
        if port == 0 || our_ssrc == 0 || remote_ssrc == 0 || payload_type > 127 {
            return None;
        }
        Some(Self {
            port,
            our_ssrc,
            remote_ssrc,
            payload_type,
        })
    }
}

#[derive(Default)]
struct AudioCounters {
    received: AtomicU64,
    reports: AtomicU64,
    rejected: AtomicU64,
    last_report_us: AtomicU64,
}

async fn drain_audio_with_feedback(
    socket: MediaSocket,
    config: MediaFeedback,
    counters: Arc<AudioCounters>,
    started: Instant,
) {
    // Protocol facts only; no implementation copied from upstream:
    // https://github.com/doronz88/pymobiledevice3/blob/master/pymobiledevice3/remote/core_device/screen_stream.py
    // Audio RTCPTimeoutInterval is typically 20s. A compound RR+SDES every
    // second is required even during silence, before the first RTP packet.
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut highest: Option<u32> = None;
    let mut sender_report: Option<(u32, Instant)> = None;
    loop {
        tokio::select! {
            biased;
            _ = interval.tick() => {
                let block = ReportBlock {
                    source_ssrc: config.remote_ssrc,
                    highest_seq: highest.unwrap_or(0),
                    lsr: sender_report.map_or(0, |(ntp, _)| ntp),
                    dlsr: sender_report.map_or(0, |(_, time)| (time.elapsed().as_secs_f64() * 65536.0).min(u32::MAX as f64) as u32),
                    ..Default::default()
                };
                // The IP is pinned by MediaSocket to the authenticated provider;
                // the destination port and SSRCs came from this audio negotiation.
                if socket.send(config.port, build_liveness(config.our_ssrc, "TetherDeck", &[block])).await.is_err() {
                    tracing::warn!("audio RTCP receiver-report transport stopped");
                    break;
                }
                counters.reports.fetch_add(1, Ordering::Relaxed);
                counters.last_report_us.store(started.elapsed().as_micros() as u64, Ordering::Relaxed);
            }
            received = socket.receive() => {
                let Ok((bytes, port)) = received else {
                    tracing::warn!("audio media receive transport stopped");
                    break;
                };
                if port != config.port { counters.rejected.fetch_add(1, Ordering::Relaxed); continue; }
                if is_rtcp(&bytes) {
                    if let Some(report) = SenderReport::parse_first(&bytes) {
                        if report.ssrc == config.remote_ssrc {
                            sender_report = Some((report.ntp_middle, Instant::now()));
                        }
                    }
                } else if let Some(packet) = RtpPacket::parse(&bytes) {
                    if packet.ssrc != config.remote_ssrc || packet.payload_type != config.payload_type { counters.rejected.fetch_add(1, Ordering::Relaxed); continue; }
                    counters.received.fetch_add(1, Ordering::Relaxed);
                    match highest {
                        None => highest = Some(u32::from(packet.sequence_number)),
                        Some(previous) => {
                            let advance = packet.sequence_number.wrapping_sub(previous as u16);
                            if advance > 0 && advance < 0x8000 {
                                highest = Some(previous.wrapping_add(u32::from(advance)));
                            }
                        }
                    }
                }
                // Audio content is neither decoded, stored, nor exposed. Only
                // the packet header counters survive this loop iteration.
            }
        }
    }
}

/// One complete Annex-B HEVC access unit. The RTP timestamp stays opaque: the
/// negotiated clock can differ across device versions. elapsed_us measures host
/// receive time, not end-to-end latency.
#[derive(Debug)]
pub struct VideoFrame {
    pub annex_b: Vec<u8>,
    pub rtp_timestamp: u32,
    pub sequence: u32,
    pub keyframe: bool,
    pub elapsed_us: u64,
}

/// Commands share the existing media session's authenticated tunnel, but run on
/// a separate task so a long drag never stops RTP reception or encoder feedback.
pub struct VideoControl {
    handshake: RsdHandshake,
    provider: Provider,
    hid: crate::hid::HidSession,
    source_size: [u32; 2],
}

impl VideoControl {
    pub fn touch_active(&self) -> bool {
        self.hid.touch_active()
    }

    pub async fn touch(&mut self, request: TouchRequest) -> Result<TouchResponse, DeviceError> {
        match &mut self.provider {
            Provider::Tunnel(provider) => {
                self.hid
                    .touch(&mut self.handshake, provider, request, self.source_size)
                    .await
            }
            Provider::Direct(provider) => {
                self.hid
                    .touch(&mut self.handshake, provider, request, self.source_size)
                    .await
            }
        }
    }

    pub async fn expire_touch(&mut self) -> Result<(), DeviceError> {
        match &mut self.provider {
            Provider::Tunnel(provider) => {
                self.hid.expire_touch(&mut self.handshake, provider).await
            }
            Provider::Direct(provider) => {
                self.hid.expire_touch(&mut self.handshake, provider).await
            }
        }
    }

    pub async fn cancel_touch(&mut self) -> Result<(), DeviceError> {
        match &mut self.provider {
            Provider::Tunnel(provider) => {
                self.hid.cancel_touch(&mut self.handshake, provider).await
            }
            Provider::Direct(provider) => {
                self.hid.cancel_touch(&mut self.handshake, provider).await
            }
        }
    }

    pub async fn force_cancel_touch(&mut self) -> Result<(), DeviceError> {
        self.cancel_touch().await
    }

    pub async fn screenshot_png(&mut self) -> Result<Vec<u8>, DeviceError> {
        tokio::time::timeout(Duration::from_secs(6), async {
            type Capture =
                idevice::services::core_device::ScreenCaptureServiceClient<Box<dyn ReadWrite>>;
            let mut capture: Capture = match &mut self.provider {
                Provider::Tunnel(provider) => self.handshake.connect(provider).await?,
                Provider::Direct(provider) => self.handshake.connect(provider).await?,
            };
            let png = capture
                .take_screenshot(None, idevice::services::core_device::ImageFormat::Png)
                .await?;
            if png.len() > 16 * 1024 * 1024 || !png.starts_with(b"\x89PNG\r\n\x1a\n") {
                return Err(DeviceError::LiveFrameUnavailable);
            }
            Ok(png)
        })
        .await
        .map_err(|_| DeviceError::Timeout)?
    }

    pub async fn action(&mut self, action: HidAction) -> Result<(), DeviceError> {
        crate::validate_hid_action(&action)?;
        // HidSession owns the 65-second execution deadline plus a separate
        // 3-second release/reset/remove phase. Do not cancel that cleanup here.
        async {
            match &mut self.provider {
                Provider::Tunnel(provider) => {
                    self.hid
                        .action(
                            &mut self.handshake,
                            provider,
                            action,
                            Some(self.source_size),
                        )
                        .await
                }
                Provider::Direct(provider) => {
                    self.hid
                        .action(
                            &mut self.handshake,
                            provider,
                            action,
                            Some(self.source_size),
                        )
                        .await
                }
            }
        }
        .await
    }
}

pub struct Video {
    /// Physical source pixels used by HID, distinct from negotiated video pixels.
    pub source_width: u32,
    pub source_height: u32,
    pub geometry: Option<HevcGeometry>,
    display: Display,
    media: MediaSocket,
    // A shared tunnel reference keeps the adapter alive after control is moved.
    _tunnel: Option<AdapterHandle>,
    audio_receiver: AudioReceiver,
    audio_feedback: MediaFeedback,
    audio_counters: Arc<AudioCounters>,
    audio_started: Instant,
    control: Option<VideoControl>,
    assembler: Assembler,
    peer_port: Option<u16>,
    media_ssrc: u32,
    payload_type: u8,
    our_ssrc: u32,
    first_seq: Option<u16>,
    highest_seq: u16,
    sequence_cycles: u32,
    received: u64,
    emitted: u32,
    discarded: u64,
    sender_report: Option<(u32, Instant)>,
    fir_sequence: u8,
    need_keyframe: bool,
    started: Instant,
    last_packet: Instant,
    last_au: Instant,
    last_ack: Option<Instant>,
    last_rtp_timestamp: u32,
    current_frame_packets: u32,
    last_frame_packets: u32,
    ack_count: u64,
    rr_count: u64,
    rctl_count: u64,
    rejected_packets: u64,
    sender_reports: u64,
    metrics_path: Option<PathBuf>,
    last_transport_metrics: Instant,
    last_report: Instant,
    last_keyframe_request: Instant,
    last_metrics: Instant,
    metrics_frames: u32,
    closed: bool,
}

impl Video {
    pub async fn connect(
        id: String,
        rsd: Option<(IpAddr, u16, String)>,
    ) -> Result<Self, DeviceError> {
        tokio::time::timeout(Duration::from_secs(25), Self::open(id, rsd))
            .await
            .map_err(|_| DeviceError::Timeout)?
    }

    async fn open(id: String, rsd: Option<(IpAddr, u16, String)>) -> Result<Self, DeviceError> {
        let (mut provider, mut handshake, media, audio, receiver, audio_port, sender, tunnel) =
            if let Some((host, port, expected)) = rsd {
                let stream = tokio::net::TcpStream::connect((host, port)).await?;
                let handshake = RsdHandshake::new(stream).await?;
                if handshake.uuid != expected {
                    return Err(DeviceError::DeviceIdMismatch);
                }
                // Pin hardware identity too when it is advertised by RSD.
                if let Some(actual) = handshake
                    .properties
                    .get("UniqueDeviceID")
                    .and_then(|v| v.as_string())
                {
                    if normalized(&id) != normalized(&expected)
                        && normalized(actual) != normalized(&id)
                    {
                        return Err(DeviceError::DeviceIdMismatch);
                    }
                }
                let wildcard: IpAddr = if host.is_ipv4() { "0.0.0.0" } else { "::" }
                    .parse()
                    .unwrap();
                let probe = tokio::net::UdpSocket::bind((wildcard, 0)).await?;
                probe.connect((host, port)).await?;
                let local = probe.local_addr()?.ip();
                let media = tokio::net::UdpSocket::bind((local, 0)).await?;
                let audio = tokio::net::UdpSocket::bind((local, 0)).await?;
                let receiver = media.local_addr()?;
                let audio_port = audio.local_addr()?.port();
                (
                    Provider::Direct(host),
                    handshake,
                    MediaSocket::Direct(media, host),
                    MediaSocket::Direct(audio, host),
                    receiver,
                    audio_port,
                    host,
                    None,
                )
            } else {
                let (tunnel, handshake) = super::usb_rsd_stack(id).await?;
                let media = tunnel.bind_udp(0).await?;
                let audio = tunnel.bind_udp(0).await?;
                let receiver = SocketAddr::new(tunnel.host_ip(), media.local_port());
                let audio_port = audio.local_port();
                let sender = tunnel.peer_ip();
                (
                    Provider::Tunnel(tunnel.clone()),
                    handshake,
                    MediaSocket::Tunnel(media),
                    MediaSocket::Tunnel(audio),
                    receiver,
                    audio_port,
                    sender,
                    Some(tunnel),
                )
            };
        let mut display: Display = match &mut provider {
            Provider::Tunnel(provider) => handshake.connect(provider).await?,
            Provider::Direct(provider) => handshake.connect(provider).await?,
        };
        let inspect_metadata = std::env::var("TETHERDECK_VIDEO_METADATA").as_deref() == Ok("1");
        if inspect_metadata {
            inspect_display_metadata(&mut handshake, &mut provider, false).await;
        }
        // Measure coordinate space once over the SAME authenticated provider.
        // HEVC may negotiate a smaller image, so decoder dimensions alone cannot
        // safely be used as HID coordinates. Failure leaves 0 (control disabled).
        let dimensions = tokio::time::timeout(Duration::from_secs(6), async {
            type Capture =
                idevice::services::core_device::ScreenCaptureServiceClient<Box<dyn ReadWrite>>;
            let mut capture: Capture = match &mut provider {
                Provider::Tunnel(provider) => handshake.connect(provider).await?,
                Provider::Direct(provider) => handshake.connect(provider).await?,
            };
            let png = capture
                .take_screenshot(None, idevice::services::core_device::ImageFormat::Png)
                .await?;
            let dimensions = if png.len() >= 24
                && png.starts_with(b"\x89PNG\r\n\x1a\n")
                && &png[12..16] == b"IHDR"
            {
                let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
                let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
                if (1..=8192).contains(&width) && (1..=8192).contains(&height) {
                    (width, height)
                } else {
                    (0, 0)
                }
            } else {
                (0, 0)
            };
            Ok::<_, DeviceError>(dimensions)
        })
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or((0, 0));
        let session_id = uuid::Uuid::new_v4();
        let call_id = uuid::Uuid::new_v4();
        let our_ssrc = uuid::Uuid::new_v4().as_u128() as u32;
        let info = CallInfoBlob {
            call_id: call_id.as_u128() as u32,
            client_version: 1,
            device_type: "Windows".into(),
            framework_version: format!("TetherDeck/{}", env!("CARGO_PKG_VERSION")),
            os_version: "Windows".into(),
            device_name: None,
            audio_device_uid: None,
        };
        // DeviceHub establishes its media-negotiator session via audio first.
        // Audio is never exposed or stored; its datagrams are drained in memory.
        let audio_start = display
            .start_media_stream(build_start_audio_parameters(
                &receiver.ip().to_string(),
                audio_port,
                &sender.to_string(),
                None,
                build_screen_audio_offer(&call_id.to_string(), &info)?,
                140,
                session_id,
            ))
            .await?;
        let Some(audio_feedback) = MediaFeedback::negotiated(&audio_start) else {
            tracing::warn!("audio negotiation omitted valid RTCP destination/SSRC metadata");
            let _ = tokio::time::timeout(Duration::from_secs(2), display.stop_media_stream()).await;
            return Err(DeviceError::LiveFrameUnavailable);
        };
        let audio_started = Instant::now();
        let audio_counters = Arc::new(AudioCounters::default());
        let audio_receiver = AudioReceiver(tokio::spawn(drain_audio_with_feedback(
            audio,
            audio_feedback,
            audio_counters.clone(),
            audio_started,
        )));
        let video_start = display
            .start_media_stream(build_start_video_parameters(
                &receiver.ip().to_string(),
                receiver.port(),
                &sender.to_string(),
                None,
                build_screen_video_offer(&call_id.to_string(), &info, our_ssrc)?,
                140,
                1,
                session_id,
            ))
            .await;
        if let Err(error) = video_start {
            let _ = tokio::time::timeout(Duration::from_secs(2), display.stop_media_stream()).await;
            return Err(error.into());
        }
        let Some(video_feedback) = video_start
            .as_ref()
            .ok()
            .and_then(MediaFeedback::negotiated)
        else {
            tracing::warn!("video negotiation omitted valid RTCP destination/SSRC metadata");
            let _ = tokio::time::timeout(Duration::from_secs(2), display.stop_media_stream()).await;
            return Err(DeviceError::LiveFrameUnavailable);
        };
        tracing::info!(
            offered_ssrc_matches_negotiated = our_ssrc == video_feedback.our_ssrc,
            negotiated_payload_type = video_feedback.payload_type,
            "video feedback uses negotiated SourcePort and SSRC identities"
        );
        if inspect_metadata {
            if let Ok(value) = &video_start {
                tracing::info!(geometry = %numeric_geometry(value), "display start geometry metadata");
            }
            inspect_display_metadata(&mut handshake, &mut provider, true).await;
        }
        let now = Instant::now();
        Ok(Self {
            source_width: dimensions.0,
            source_height: dimensions.1,
            geometry: None,
            display,
            media,
            _tunnel: tunnel,
            audio_receiver,
            audio_feedback,
            audio_counters,
            audio_started,
            control: Some(VideoControl {
                handshake,
                provider,
                hid: crate::hid::HidSession::default(),
                source_size: [dimensions.0, dimensions.1],
            }),
            assembler: Assembler::default(),
            peer_port: Some(video_feedback.port),
            media_ssrc: video_feedback.remote_ssrc,
            payload_type: video_feedback.payload_type,
            our_ssrc: video_feedback.our_ssrc,
            first_seq: None,
            highest_seq: 0,
            sequence_cycles: 0,
            received: 0,
            emitted: 0,
            discarded: 0,
            sender_report: None,
            fir_sequence: 0,
            need_keyframe: true,
            started: now,
            last_packet: now,
            last_au: now,
            last_ack: None,
            last_rtp_timestamp: 0,
            current_frame_packets: 0,
            last_frame_packets: 0,
            ack_count: 0,
            rr_count: 0,
            rctl_count: 0,
            rejected_packets: 0,
            sender_reports: 0,
            metrics_path: std::env::var_os("TETHERDECK_RTP_METRICS")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
            last_transport_metrics: now,
            last_report: now,
            last_keyframe_request: now - Duration::from_secs(2),
            last_metrics: now,
            metrics_frames: 0,
            closed: false,
        })
    }

    pub fn take_control(&mut self) -> Option<VideoControl> {
        self.control.take()
    }

    /// A slow consumer may discard one access unit only if every dependent
    /// frame is then suppressed until a fresh random-access picture arrives.
    pub fn request_resync(&mut self) {
        self.need_keyframe = true;
        self.assembler.complete.clear();
        self.last_keyframe_request = Instant::now() - Duration::from_secs(2);
        self.discarded += 1;
    }

    /// Keeps reading until one complete decodable access unit is available.
    /// No frame-rate timer or PNG/JPEG conversion exists on the hot path.
    pub async fn frame(&mut self) -> Result<VideoFrame, DeviceError> {
        loop {
            let packet = tokio::select! {
                packet = self.media.receive() => Some(packet?),
                _ = tokio::time::sleep(Duration::from_millis(12)) => None,
            };
            if let Some((bytes, port)) = packet {
                // Feedback goes to the negotiated SourcePort, not whichever
                // source port was last observed. Peer IP is pinned by the socket.
                if self.peer_port != Some(port) {
                    self.rejected_packets += 1;
                    continue;
                }
                if is_rtcp(&bytes) {
                    if let Some(report) = SenderReport::parse_first(&bytes) {
                        if report.ssrc != self.media_ssrc {
                            self.rejected_packets += 1;
                            continue;
                        }
                        self.last_packet = Instant::now();
                        self.sender_reports += 1;
                        self.sender_report = Some((report.ntp_middle, Instant::now()));
                    }
                } else if let Some(packet) = RtpPacket::parse(&bytes) {
                    if packet.payload_type != self.payload_type {
                        self.rejected_packets += 1;
                        continue;
                    }
                    if self.media_ssrc != packet.ssrc {
                        self.rejected_packets += 1;
                        continue;
                    }
                    self.last_packet = Instant::now();
                    self.received += 1;
                    self.last_rtp_timestamp = packet.timestamp;
                    self.current_frame_packets += 1;
                    if packet.marker {
                        self.last_frame_packets = self.current_frame_packets;
                        self.current_frame_packets = 0;
                        // Receiver credits follow observed frame boundaries,
                        // including frames later rejected by loss recovery.
                        // Waiting for HEVC reassembly withheld credits on loss.
                        self.media
                            .send(port, build_frame_ack(self.our_ssrc, packet.timestamp))
                            .await?;
                        self.ack_count += 1;
                        self.last_ack = Some(Instant::now());
                    }
                    if self.first_seq.is_none() {
                        self.first_seq = Some(packet.sequence_number);
                        self.highest_seq = packet.sequence_number;
                    } else if packet.sequence_number.wrapping_sub(self.highest_seq) < 0x8000 {
                        if packet.sequence_number < self.highest_seq {
                            self.sequence_cycles += 1 << 16;
                        }
                        self.highest_seq = packet.sequence_number;
                    }
                    self.assembler.push(&packet);
                }
            }
            if self.last_packet.elapsed() > Duration::from_secs(15) {
                return Err(DeviceError::Timeout);
            }
            if self.assembler.expire_gap() {
                self.need_keyframe = true;
                self.discarded += 1;
            }
            self.feedback().await?;
            if let Some((timestamp, annex_b, keyframe)) = self.assembler.next_frame() {
                if self.need_keyframe && !keyframe {
                    self.discarded += 1;
                    continue;
                }
                if keyframe {
                    self.need_keyframe = false;
                    if let Some(geometry) = sps::geometry(&annex_b) {
                        if self.geometry.is_none() {
                            tracing::info!(
                                ?geometry,
                                source_width = self.source_width,
                                source_height = self.source_height,
                                "HEVC SPS geometry (does not infer content padding)"
                            );
                        }
                        self.geometry = Some(geometry);
                    }
                }
                self.emitted = self.emitted.wrapping_add(1);
                self.last_au = Instant::now();
                self.metrics_frames += 1;
                if self.last_metrics.elapsed() >= Duration::from_secs(5) {
                    tracing::info!(
                        source_fps =
                            self.metrics_frames as f64 / self.last_metrics.elapsed().as_secs_f64(),
                        received_packets = self.received,
                        discarded_frames = self.discarded,
                        "encoded video receive metrics (not display FPS)"
                    );
                    self.metrics_frames = 0;
                    self.last_metrics = Instant::now();
                }
                return Ok(VideoFrame {
                    annex_b,
                    rtp_timestamp: timestamp,
                    sequence: self.emitted,
                    keyframe,
                    elapsed_us: self.started.elapsed().as_micros() as u64,
                });
            }
        }
    }

    async fn feedback(&mut self) -> Result<(), DeviceError> {
        let Some(port) = self.peer_port else {
            return Ok(());
        };
        let now = Instant::now();
        let report = ReportBlock {
            source_ssrc: self.media_ssrc,
            highest_seq: self.sequence_cycles | u32::from(self.highest_seq),
            cumulative_lost: self.assembler.lost_packets.min(0x7fffff) as u32,
            lsr: self.sender_report.map_or(0, |(ntp, _)| ntp),
            dlsr: self.sender_report.map_or(0, |(_, time)| {
                (time.elapsed().as_secs_f64() * 65536.0) as u32
            }),
            ..Default::default()
        };
        if self.last_report.elapsed() >= Duration::from_millis(50) {
            self.media
                .send(
                    port,
                    receiver_control(
                        self.our_ssrc,
                        self.last_rtp_timestamp,
                        self.last_frame_packets,
                        (self.last_rtp_timestamp / (RCTL_CLOCK_ASSUMPTION_HZ / 1000)) as u16,
                        self.received as u16,
                    ),
                )
                .await?;
            self.rctl_count += 1;
            // RR/SDES are cheap and accompany Apple's receiver-control reports.
            self.media
                .send(port, build_liveness(self.our_ssrc, "TetherDeck", &[report]))
                .await?;
            self.rr_count += 1;
            self.last_report = now;
        }
        if self.need_keyframe && self.last_keyframe_request.elapsed() >= Duration::from_millis(700)
        {
            self.fir_sequence = self.fir_sequence.wrapping_add(1);
            self.media
                .send(
                    port,
                    build_keyframe_request(
                        self.our_ssrc,
                        "TetherDeck",
                        self.media_ssrc,
                        &[report],
                        self.fir_sequence,
                    ),
                )
                .await?;
            self.last_keyframe_request = now;
        }
        if self.last_transport_metrics.elapsed() >= Duration::from_secs(5) {
            self.write_transport_metrics(false).await;
            self.last_transport_metrics = Instant::now();
        }
        Ok(())
    }

    async fn write_transport_metrics(&self, ended: bool) {
        let Some(path) = &self.metrics_path else {
            return;
        };
        let audio_elapsed_us = self.audio_started.elapsed().as_micros() as u64;
        let audio_rr_us = self.audio_counters.last_report_us.load(Ordering::Relaxed);
        let value = serde_json::json!({
            "ended": u8::from(ended),
            "elapsed_us": self.started.elapsed().as_micros() as u64,
            "video_source_port": self.peer_port.unwrap_or(0),
            "video_host_ssrc": self.our_ssrc,
            "video_device_ssrc": self.media_ssrc,
            "video_payload_type": self.payload_type,
            "video_received_packets": self.received,
            "video_emitted_units": self.emitted,
            "video_discarded_units": self.discarded,
            "video_rejected_packets": self.rejected_packets,
            "video_sender_reports": self.sender_reports,
            "video_ack_count": self.ack_count,
            "video_rr_count": self.rr_count,
            "video_rctl_count": self.rctl_count,
            "video_highest_sequence": self.sequence_cycles | u32::from(self.highest_seq),
            "video_last_rtp_timestamp": self.last_rtp_timestamp,
            "video_last_frame_packets": self.last_frame_packets,
            "rctl_clock_assumption_hz": RCTL_CLOCK_ASSUMPTION_HZ,
            "video_last_packet_age_us": self.last_packet.elapsed().as_micros() as u64,
            "video_last_au_age_us": self.last_au.elapsed().as_micros() as u64,
            "video_has_ack": u8::from(self.last_ack.is_some()),
            "video_last_ack_age_us": self.last_ack.map_or(0, |time| time.elapsed().as_micros() as u64),
            "video_last_rr_age_us": self.last_report.elapsed().as_micros() as u64,
            "audio_source_port": self.audio_feedback.port,
            "audio_host_ssrc": self.audio_feedback.our_ssrc,
            "audio_device_ssrc": self.audio_feedback.remote_ssrc,
            "audio_payload_type": self.audio_feedback.payload_type,
            "audio_received_packets": self.audio_counters.received.load(Ordering::Relaxed),
            "audio_rejected_packets": self.audio_counters.rejected.load(Ordering::Relaxed),
            "audio_rr_count": self.audio_counters.reports.load(Ordering::Relaxed),
            "audio_last_rr_age_us": audio_elapsed_us.saturating_sub(audio_rr_us)
        });
        if let Ok(bytes) = serde_json::to_vec(&value) {
            // Opt-in, content-free diagnostics must not block RTP indefinitely.
            let _ = tokio::time::timeout(Duration::from_millis(150), tokio::fs::write(path, bytes))
                .await;
        }
    }

    pub async fn close(&mut self) {
        if !self.closed {
            self.closed = true;
            self.write_transport_metrics(true).await;
            let _ = tokio::time::timeout(Duration::from_secs(2), self.display.stop_media_stream())
                .await;
            self.audio_receiver.0.abort();
        }
    }
}

fn receiver_control(
    our_ssrc: u32,
    rtp_timestamp: u32,
    last_frame_packets: u32,
    arrival_ms: u16,
    received_packets: u16,
) -> Vec<u8> {
    // Wire field facts from the primary reverse-engineered protocol description,
    // not copied implementation: pymobiledevice3 remote/core_device/screen_stream.py
    // _build_rctl_packet. The earlier vendor helper placed frame counts in the
    // RTP-timestamp word and left the frame packet-count word empty.
    let words = [
        0x80cc0007u32,
        our_ssrc,
        u32::from_be_bytes(*b"RCTL"),
        0x85000004,
        ((rtp_timestamp >> 8) & 0xffff) << 16,
        last_frame_packets,
        u32::from(arrival_ms) << 16,
        (u32::from(received_packets) << 16) | 0xea61,
    ];
    words.into_iter().flat_map(u32::to_be_bytes).collect()
}

impl Drop for Video {
    fn drop(&mut self) {
        self.audio_receiver.0.abort();
        // Explicit close() performs asynchronous remote cleanup. Dropping the
        // display transport is the cancellation backstop (no detached stopAll).
    }
}

fn normalized(id: &str) -> String {
    id.chars()
        .filter(|c| *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

struct Packet {
    seq: u16,
    timestamp: u32,
    marker: bool,
    payload: Vec<u8>,
}

#[derive(Default)]
struct Assembler {
    queued: Vec<Packet>,
    expected: Option<u16>,
    gap_since: Option<Instant>,
    depacketizer: HevcDepacketizer,
    timestamp: Option<u32>,
    payload_bytes: usize,
    skip_timestamp: Option<u32>,
    current: Vec<u8>,
    complete: std::collections::VecDeque<(u32, Vec<u8>, bool)>,
    lost_packets: u64,
    damaged: bool,
}

impl Assembler {
    fn push(&mut self, packet: &RtpPacket<'_>) {
        let expected = *self.expected.get_or_insert(packet.sequence_number);
        let distance = packet.sequence_number.wrapping_sub(expected);
        if distance >= 0x8000 || self.queued.iter().any(|p| p.seq == packet.sequence_number) {
            return;
        }
        self.queued.push(Packet {
            seq: packet.sequence_number,
            timestamp: packet.timestamp,
            marker: packet.marker,
            payload: packet.payload.to_vec(),
        });
        if distance != 0 {
            self.gap_since.get_or_insert_with(Instant::now);
        }
        self.drain();
    }

    fn expire_gap(&mut self) -> bool {
        let expired = self.queued.len() > REORDER_PACKETS
            || self.gap_since.is_some_and(|t| t.elapsed() >= REORDER_WAIT);
        if !expired || self.queued.is_empty() {
            return std::mem::take(&mut self.damaged);
        }
        let expected = self.expected.unwrap();
        let next = self
            .queued
            .iter()
            .min_by_key(|p| p.seq.wrapping_sub(expected))
            .unwrap()
            .seq;
        self.lost_packets += u64::from(next.wrapping_sub(expected));
        self.expected = Some(next);
        self.depacketizer = HevcDepacketizer::new();
        self.current.clear();
        self.timestamp = None;
        self.payload_bytes = 0;
        self.skip_timestamp = None;
        self.gap_since = None;
        self.drain();
        true
    }

    fn drain(&mut self) {
        while let Some(index) = self
            .queued
            .iter()
            .position(|p| Some(p.seq) == self.expected)
        {
            let packet = self.queued.swap_remove(index);
            self.expected = Some(packet.seq.wrapping_add(1));
            if self.skip_timestamp == Some(packet.timestamp) {
                continue;
            }
            if self.timestamp != Some(packet.timestamp) {
                self.payload_bytes = 0;
                self.skip_timestamp = None;
            }
            if self
                .timestamp
                .is_some_and(|timestamp| timestamp != packet.timestamp)
                && contains_vcl(&self.current)
            {
                // A missing marker or damaged access unit is never sent to decoder.
                self.current.clear();
                self.depacketizer = HevcDepacketizer::new();
                self.damaged = true;
            }
            self.timestamp = Some(packet.timestamp);
            self.payload_bytes = self.payload_bytes.saturating_add(packet.payload.len());
            if self.payload_bytes > MAX_ACCESS_UNIT {
                // Bound fragment storage inside the vendor depacketizer too.
                self.current.clear();
                self.depacketizer = HevcDepacketizer::new();
                self.skip_timestamp = Some(packet.timestamp);
                self.damaged = true;
                continue;
            }
            self.depacketizer
                .push(packet.seq, packet.timestamp, &packet.payload);
            self.current.extend(self.depacketizer.take_output());
            if self.current.len() > MAX_ACCESS_UNIT {
                self.current.clear();
                self.depacketizer = HevcDepacketizer::new();
                self.damaged = true;
            }
            if packet.marker && contains_vcl(&self.current) {
                let annex_b = std::mem::take(&mut self.current);
                let keyframe = contains_irap(&annex_b) && self.depacketizer.has_parameter_sets();
                if self.complete.len() >= 3 {
                    self.complete.clear();
                    self.damaged = true;
                }
                self.complete
                    .push_back((packet.timestamp, annex_b, keyframe));
            }
        }
        if self.queued.is_empty() {
            self.gap_since = None;
        }
    }

    fn next_frame(&mut self) -> Option<(u32, Vec<u8>, bool)> {
        self.complete.pop_front()
    }
}

fn contains_irap(bytes: &[u8]) -> bool {
    bytes
        .windows(5)
        .any(|w| w[..4] == [0, 0, 0, 1] && (16..=23).contains(&((w[4] >> 1) & 0x3f)))
}

fn contains_vcl(bytes: &[u8]) -> bool {
    bytes
        .windows(5)
        .any(|w| w[..4] == [0, 0, 0, 1] && ((w[4] >> 1) & 0x3f) <= 31)
}

async fn inspect_display_metadata(
    handshake: &mut RsdHandshake,
    provider: &mut Provider,
    status: bool,
) {
    // An optional diagnostic connection uses the same tunnel, but its own XPC
    // stream so timing out a query cannot poison the media control channel.
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        let mut metadata: Display = match provider {
            Provider::Tunnel(provider) => handshake.connect(provider).await?,
            Provider::Direct(provider) => handshake.connect(provider).await?,
        };
        if status {
            metadata.get_media_stream_server_status().await
        } else {
            metadata.get_media_support_info().await
        }
    })
    .await;
    match result {
        Ok(Ok(value)) => {
            tracing::info!(status, geometry=%numeric_geometry(&value), "display geometry metadata")
        }
        _ => tracing::info!(status, "optional display geometry metadata unavailable"),
    }
}

fn numeric_geometry(value: &plist::Value) -> serde_json::Value {
    fn collect(
        value: &plist::Value,
        path: &str,
        depth: usize,
        result: &mut serde_json::Map<String, serde_json::Value>,
    ) {
        if depth > 10 || result.len() >= 128 {
            return;
        }
        match value {
            plist::Value::Dictionary(dictionary) => {
                for (key, value) in dictionary {
                    if key.len() <= 80 && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    {
                        let path = if path.is_empty() {
                            key.clone()
                        } else {
                            format!("{path}.{key}")
                        };
                        collect(value, &path, depth + 1, result);
                    }
                }
            }
            plist::Value::Array(values) => {
                for (index, value) in values.iter().take(16).enumerate() {
                    collect(value, &format!("{path}[{index}]"), depth + 1, result);
                }
            }
            _ => {
                let name = path.to_ascii_lowercase();
                if ![
                    "width",
                    "height",
                    "size",
                    "crop",
                    "rect",
                    "origin",
                    "resolution",
                    "orientation",
                    "framerate",
                    "dimension",
                    "scale",
                ]
                .iter()
                .any(|key| name.contains(key))
                {
                    return;
                }
                let number = match value {
                    plist::Value::Integer(number) => number
                        .as_signed()
                        .map(serde_json::Value::from)
                        .or_else(|| number.as_unsigned().map(serde_json::Value::from)),
                    plist::Value::Real(number) if number.is_finite() => {
                        Some(serde_json::json!(number))
                    }
                    _ => None,
                };
                if let Some(number) = number {
                    result.insert(path.to_owned(), number);
                }
            }
        }
    }
    let mut result = serde_json::Map::new();
    collect(value, "", 0, &mut result);
    result.into()
}
