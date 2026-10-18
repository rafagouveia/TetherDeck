//! GPUI native remote display. Device I/O and pixel conversion stay on Tokio.
use crate::{backend::Session, desktop::theme, window_chrome};
use crate::i18n::tr;
use device_video::DecodedFrame;
use gpui_kit::assets::IconName;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    component::{Disableable, Selectable, button::Button},
    *,
};
use image::ImageEncoder;
use rust_ios_device_api::{DeviceButton, HidAction, TouchPhase, TouchRequest};
use std::{
    cell::Cell,
    collections::VecDeque,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{oneshot, watch};

const HORIZONTAL_CHROME: f32 = 88.;
const VERTICAL_CHROME: f32 = 76.;

fn control_button(id: impl Into<ElementId>) -> Button {
    crate::desktop::button(id).px(px(0.))
}

#[derive(Clone, Copy, PartialEq)]
struct Geometry {
    physical: [usize; 2],
    coded: [usize; 2],
    scaled: [usize; 2],
    crop: [usize; 2],
    uv: [f32; 2],
}

/// Only accept a source-sized frame or small, dark alignment padding. Decoder
/// downscaling is explicit metadata, never inferred from the phone model.
fn geometry(frame: &DecodedFrame) -> Option<Geometry> {
    let source = frame.source_size;
    let coded = frame.coded_size;
    let scaled = frame.size;
    if source.iter().any(|&n| n == 0 || n > u16::MAX as usize)
        || coded.iter().any(|&n| n == 0 || n > 4096)
        || scaled.iter().any(|&n| n == 0 || n > 4096)
        || frame.rgba.len() != scaled[0].checked_mul(scaled[1])?.checked_mul(4)?
        || scaled[0] > coded[0]
        || scaled[1] > coded[1]
    {
        return None;
    }
    // scale=...:-2 rounds the output height to an even integer.
    let expected_height = scaled[0] as f64 * coded[1] as f64 / coded[0] as f64;
    if (scaled[1] as f64 - expected_height).abs() > 2.0 {
        return None;
    }
    for axis in 0..2 {
        if coded[axis] < source[axis]
            || coded[axis] - source[axis] >= 64
            || (coded[axis] != source[axis]
                && ![16usize, 64]
                    .iter()
                    .any(|&a| source[axis].div_ceil(a) * a == coded[axis]))
        {
            return None;
        }
    }
    let dark = |x: usize, y: usize| {
        let offset = (y * scaled[0] + x) * 4;
        frame.rgba[offset + 3] >= 250 && frame.rgba[offset..offset + 3].iter().all(|&c| c <= 64)
    };
    // Ignore the first resampled boundary pixel, which can blend bright content.
    if coded[0] > source[0] {
        let first = (source[0] * scaled[0]).div_ceil(coded[0]) + 1;
        if first >= scaled[0] {
            return None;
        }
        for x in first..scaled[0] {
            for i in 0..=32 {
                if !dark(x, i * (scaled[1] - 1) / 32) {
                    return None;
                }
            }
        }
    }
    if coded[1] > source[1] {
        let first = (source[1] * scaled[1]).div_ceil(coded[1]) + 1;
        if first >= scaled[1] {
            return None;
        }
        for y in first..scaled[1] {
            for i in 0..=32 {
                if !dark(i * (scaled[0] - 1) / 32, y) {
                    return None;
                }
            }
        }
    }
    Some(Geometry {
        physical: source,
        coded,
        scaled,
        crop: [
            source[0] * scaled[0] / coded[0],
            source[1] * scaled[1] / coded[1],
        ],
        uv: [
            source[0] as f32 / coded[0] as f32,
            source[1] as f32 / coded[1] as f32,
        ],
    })
}

struct Frame {
    image: Arc<RenderImage>,
    geometry: Option<Geometry>,
    size: [usize; 2],
    sequence: u64,
    received: Instant,
}
#[derive(Clone)]
enum VideoState {
    Starting,
    Frame(Arc<Frame>),
    Reconnecting { reason: String, delay_ms: u64 },
    Ended(String),
}

#[derive(Clone)]
struct VideoEvent {
    generation: u64,
    attempt: u64,
    state: VideoState,
}

struct InputInFlight(Arc<AtomicU64>);
impl InputInFlight {
    fn new(count: Arc<AtomicU64>) -> Self {
        count.fetch_add(1, Ordering::SeqCst);
        Self(count)
    }
}
impl Drop for InputInFlight {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// One sequential supervisor owns every HTTP/FFmpeg incarnation. A retry is
/// only a new read-only viewer; input commands and pointer samples never replay.
async fn supervise_video(
    session: Arc<Session>,
    send: watch::Sender<VideoEvent>,
    mut cancel: watch::Receiver<bool>,
    generation: u64,
    inputs: Arc<AtomicU64>,
    previous: Option<tokio::task::JoinHandle<()>>,
) {
    let had_previous = previous.is_some();
    if let Some(previous) = previous {
        let _ = previous.await;
    }
    let mut attempt = 0u64;
    let mut failures = 0u32;
    loop {
        if *cancel.borrow() || send.is_closed() {
            return;
        }
        // UI cancellation never aborts a mutation. Await its release worker,
        // then allow a full touch lease to elapse before opening another viewer.
        let release_deadline = Instant::now() + Duration::from_secs(70);
        while inputs.load(Ordering::SeqCst) != 0 {
            if Instant::now() >= release_deadline {
                let _ = send.send(VideoEvent { generation, attempt, state: VideoState::Ended("O controle anterior não confirmou o término. Confira o aparelho antes de retomar.".into()) });
                return;
            }
            tokio::select! { biased;
                _ = cancel.changed() => return,
                _ = tokio::time::sleep(Duration::from_millis(20)) => {},
            }
        }
        if attempt > 0 || had_previous {
            tokio::select! { biased;
                _ = cancel.changed() => return,
                _ = tokio::time::sleep(Duration::from_millis(850)) => {},
            }
        }
        if *cancel.borrow() || send.is_closed() {
            return;
        }
        attempt += 1;
        let _ = send.send(VideoEvent {
            generation,
            attempt,
            state: VideoState::Starting,
        });
        let mut cached: Option<Geometry> = None;
        let mut stable_since: Option<Instant> = None;
        let mut last_frame: Option<Instant> = None;
        let mut stable = false;
        let mut had_frame = false;
        let result = device_video::run_cancellable(
            session.client.clone(),
            session.device_id.clone(),
            |mut raw| {
                if *cancel.borrow() || send.is_closed() {
                    return false;
                }
                let now = Instant::now();
                if last_frame
                    .is_none_or(|previous| now.duration_since(previous) >= Duration::from_secs(2))
                {
                    stable_since = Some(now);
                }
                last_frame = Some(now);
                stable |= stable_since
                    .is_some_and(|since| now.duration_since(since) >= Duration::from_secs(60));
                had_frame = true;
                let shape = (raw.source_size, raw.coded_size, raw.size);
                let mapping = cached
                    .filter(|old| (old.physical, old.coded, old.scaled) == shape)
                    .or_else(|| geometry(&raw));
                cached = mapping;
                for pixel in raw.rgba.chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                }
                let Some(buffer) =
                    image::RgbaImage::from_raw(raw.size[0] as u32, raw.size[1] as u32, raw.rgba)
                else {
                    return false;
                };
                let image = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
                send.send(VideoEvent {
                    generation,
                    attempt,
                    state: VideoState::Frame(Arc::new(Frame {
                        image,
                        geometry: mapping,
                        size: raw.size,
                        sequence: raw.sequence,
                        received: raw.received_at,
                    })),
                })
                .is_ok()
            },
            cancel.clone(),
        )
        .await;
        if *cancel.borrow() || send.is_closed() {
            return;
        }
        let reason = match result {
            Ok(()) => {
                let _ = send.send(VideoEvent {
                    generation,
                    attempt,
                    state: VideoState::Ended(
                        "O vídeo encerrou sem um quadro válido. Consulte o diagnóstico de vídeo."
                            .into(),
                    ),
                });
                return;
            }
            Err(reason) => reason,
        };
        // A decoder that immediately exits even in software mode is a setup/
        // codec failure. A previously working decoder EOF is recoverable.
        let unsupported_decoder = !had_frame
            && (reason.starts_with("O decoder encerrou o vídeo") || reason == "Decoder encerrado.");
        if !device_video::retryable_failure(&reason) || unsupported_decoder {
            let _ = send.send(VideoEvent {
                generation,
                attempt,
                state: VideoState::Ended(reason),
            });
            return;
        }
        if stable {
            failures = 0;
        }
        failures = failures.saturating_add(1);
        let base = 500u64.saturating_mul(1u64 << failures.saturating_sub(1).min(5));
        let jitter = uuid::Uuid::new_v4().as_u128() as u64 % 151;
        let delay_ms = base.saturating_add(jitter).min(10_000);
        let _ = send.send(VideoEvent {
            generation,
            attempt,
            state: VideoState::Reconnecting {
                reason: reason.clone(),
                delay_ms,
            },
        });
        if let Some(path) =
            std::env::var_os("TETHERDECK_RECOVERY_METRICS").filter(|path| !path.is_empty())
        {
            let code = reason
                .split(':')
                .next()
                .filter(|code| {
                    code.len() <= 64
                        && code
                            .bytes()
                            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
                })
                .unwrap_or("decoder_or_transport");
            let value = serde_json::json!({"generation":generation,"attempt":attempt,"consecutive_failures":failures,"next_retry_ms":delay_ms,"stable_seconds_required":60,"failure_code":code});
            let _ = tokio::time::timeout(
                Duration::from_millis(100),
                tokio::fs::write(path, value.to_string()),
            )
            .await;
        }
        tokio::select! { biased;
            _ = cancel.changed() => return,
            _ = tokio::time::sleep(Duration::from_millis(delay_ms)) => {},
        }
    }
}

#[derive(Default)]
struct PaintMetrics {
    count: AtomicU64,
    sequence: AtomicU64,
}

#[derive(Clone, Copy)]
enum TouchIntent {
    Held((u16, u16)),
    End((u16, u16)),
    Cancel((u16, u16)),
}

struct Drag {
    point: (u16, u16),
    geometry: Geometry,
    intent: watch::Sender<TouchIntent>,
}

async fn touch_sample(session: &Session, request: &TouchRequest) -> Result<u64, String> {
    let started = Instant::now();
    let response = tokio::time::timeout(
        Duration::from_millis(500),
        session.client.touch(&session.device_id, request),
    )
    .await
    .map_err(|_| "Resposta do toque indisponível; controle suspenso por segurança.".to_string())?
    .map_err(|error| error.to_string())?;
    let expected_active = matches!(request.phase, TouchPhase::Start | TouchPhase::Move);
    if response.gesture_id != request.gesture_id
        || response.sequence != request.sequence
        || response.active != expected_active
    {
        return Err("Resposta do toque não corresponde ao gesto enviado.".into());
    }
    Ok(started.elapsed().as_millis() as u64)
}

// One OS worker owns Runtime and Session until the final release. No UI task
// cancellation can abort this future, and no failed sample is replayed.
async fn live_touch(
    session: &Session,
    mut intent: watch::Receiver<TouchIntent>,
    gesture_id: String,
    initial: (u16, u16),
) -> (Result<(), String>, u64) {
    let mut request = TouchRequest {
        gesture_id,
        sequence: 1,
        phase: TouchPhase::Start,
        x: initial.0,
        y: initial.1,
    };
    let mut last_ms = 0;
    let result = async {
        last_ms = touch_sample(session, &request).await?;
        let mut last_point = initial;
        let mut sent_at = tokio::time::Instant::now();
        loop {
            // Terminal intent discards unsent intermediate points. End may need
            // one final CONTACT at its endpoint; Cancel never adds a Move.
            let latest = *intent.borrow_and_update();
            let point = match latest {
                TouchIntent::End(point) | TouchIntent::Cancel(point) => {
                    if matches!(latest, TouchIntent::End(_)) && point != last_point {
                        request.sequence += 1;
                        request.phase = TouchPhase::Move;
                        request.x = point.0;
                        request.y = point.1;
                        last_ms = touch_sample(session, &request).await?;
                    }
                    // A focus/stale/close cancellation can arrive during the
                    // final CONTACT. Release as Cancel rather than End then.
                    let terminal = *intent.borrow_and_update();
                    let (phase, point) = match terminal {
                        TouchIntent::Cancel(point) => (TouchPhase::Cancel, point),
                        _ if matches!(latest, TouchIntent::Cancel(_)) => {
                            (TouchPhase::Cancel, point)
                        }
                        _ => (TouchPhase::End, point),
                    };
                    request.sequence += 1;
                    request.phase = phase;
                    request.x = point.0;
                    request.y = point.1;
                    last_ms = touch_sample(session, &request).await?;
                    return Ok(());
                }
                TouchIntent::Held(point) => point,
            };
            let due = sent_at
                + if point != last_point {
                    Duration::from_millis(34)
                } else {
                    Duration::from_millis(150)
                };
            if tokio::time::Instant::now() < due {
                tokio::select! {
                    changed = intent.changed() => {
                        if changed.is_err() {
                            return Err("Gesto interrompido; liberação solicitada.".into());
                        }
                    }
                    _ = tokio::time::sleep_until(due) => {}
                }
                continue;
            }
            request.sequence += 1;
            request.phase = TouchPhase::Move;
            request.x = point.0;
            request.y = point.1;
            last_ms = touch_sample(session, &request).await?;
            last_point = point;
            sent_at = tokio::time::Instant::now();
        }
    }
    .await;
    if result.is_err() {
        // A timeout is an unknown outcome, not permission to repeat a sample.
        // Attempt only release; the daemon's lease also bounds a lost connection.
        request.sequence += 1;
        request.phase = TouchPhase::Cancel;
        if touch_sample(session, &request).await.is_err() {
            eprintln!("TetherDeck: confirmação de liberação do toque indisponível");
        }
    }
    if let Some(path) = std::env::var_os("TETHERDECK_INPUT_METRICS").filter(|p| !p.is_empty()) {
        let value = serde_json::json!({"backend":"gpui","action_kind":"touch","phase":request.phase,"command_response_ms":last_ms,"sample_count":request.sequence,"measurement":"Last live-touch API request to response; not end-to-end interaction or video latency"});
        let _ = tokio::fs::write(path, value.to_string()).await;
    }
    (result, last_ms)
}

pub struct Remote {
    session: Arc<Session>,
    runtime: Arc<tokio::runtime::Runtime>,
    stream: Option<tokio::task::JoinHandle<()>>,
    stream_cancel: Option<watch::Sender<bool>>,
    retiring_stream: Option<tokio::task::JoinHandle<()>>,
    input_in_flight: Arc<AtomicU64>,
    generation: u64,
    video_attempt: u64,
    viewing_requested: bool,
    recovering: bool,
    recovery_geometry: Option<Geometry>,
    recovery_attempts: u64,
    recovery_count: u64,
    recovery_wait_ms: u64,
    recovery_reason: Option<String>,
    pump: Option<Task<()>>,
    stale_task: Option<Task<()>>,
    frame: Option<Arc<Frame>>,
    paint_metrics: Arc<PaintMetrics>,
    samples: VecDeque<(Instant, u64)>,
    command_task: Option<Task<()>>,
    touch_reply: Option<Task<()>>,
    activation: Option<Subscription>,
    busy: bool,
    paused: bool,
    // Intent survives a temporary video delay; readiness is checked per input.
    control_requested: bool,
    video_stale: bool,
    auto_control_pending: bool,
    pointer_feedback: Option<(Point<Pixels>, Instant)>,
    message: String,
    action_error: Option<String>,
    command_ms: Option<u64>,
    drag: Option<Drag>,
    // Retained through pending End so focus/close can still request Cancel.
    touch_intent: Option<watch::Sender<TouchIntent>>,
    image_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    canvas_size: Rc<Cell<Option<Size<Pixels>>>>,
    started: Instant,
    last_metrics: Instant,
    chrome_aspect: Option<f32>,
    show_stats: bool,
}
impl Drop for Remote {
    fn drop(&mut self) {
        self.cancel_touch();
        if let Some(cancel) = self.stream_cancel.take() {
            let _ = cancel.send(true);
        }
        self.write_present_metrics(true);
        // In-flight command tasks run independently on Tokio and retain Session
        // until the device has completed release/end. Never abort a mutation.
    }
}

pub fn open(
    session: Arc<Session>,
    runtime: Arc<tokio::runtime::Runtime>,
    cx: &mut App,
) -> anyhow::Result<AnyWindowHandle> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(440.), px(840.)),
            cx,
        ))),
        window_min_size: Some(size(px(360.), px(440.))),
        window_background: WindowBackgroundAppearance::Transparent,
        titlebar: Some(TitlebarOptions {
            title: Some(format!("{} · TetherDeck", crate::desktop::presentation_device_name(&session.name)).into()),
            appears_transparent: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    let handle = cx.open_window(options, |window, cx| {
        let remote = cx.new(|cx| Remote::new(session, runtime, window, cx));
        // Component plugins supply an opaque default. Instance styling takes
        // precedence, so the native transparent surface stays transparent.
        cx.new(|cx| gpui_kit::base::Root::new(remote, window, cx).bg(transparent_black()))
    })?;
    Ok(handle.into())
}

impl Remote {
    fn new(
        session: Arc<Session>,
        runtime: Arc<tokio::runtime::Runtime>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        if let Err(error) =
            window_chrome::configure(window, 9. / 19.5, HORIZONTAL_CHROME, VERTICAL_CHROME)
        {
            eprintln!("TetherDeck: decoração da janela remota: {error}");
        }
        let mut remote = Self {
            session,
            runtime,
            stream: None,
            stream_cancel: None,
            retiring_stream: None,
            input_in_flight: Arc::new(AtomicU64::new(0)),
            generation: 0,
            video_attempt: 0,
            viewing_requested: false,
            recovering: false,
            recovery_geometry: None,
            recovery_attempts: 0,
            recovery_count: 0,
            recovery_wait_ms: 0,
            recovery_reason: None,
            pump: None,
            stale_task: None,
            frame: None,
            paint_metrics: Arc::new(PaintMetrics::default()),
            samples: VecDeque::new(),
            command_task: None,
            touch_reply: None,
            activation: None,
            busy: false,
            paused: false,
            control_requested: false,
            video_stale: true,
            auto_control_pending: true,
            pointer_feedback: None,
            message: "Abrindo transmissão…".into(),
            action_error: None,
            command_ms: None,
            drag: None,
            touch_intent: None,
            image_bounds: Rc::new(Cell::new(None)),
            canvas_size: Rc::new(Cell::new(None)),
            started: Instant::now(),
            last_metrics: Instant::now(),
            chrome_aspect: None,
            show_stats: false,
        };
        remote.activation = Some(cx.observe_window_activation(window, |view, window, cx| {
            if !window.is_window_active() {
                view.cancel_touch();
                cx.notify();
            }
        }));
        remote.start(true, window, cx);
        remote.stale_task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                if this
                    .update_in(cx, |view, _, cx| {
                        let stale = !view.fresh();
                        if view.video_stale != stale {
                            view.video_stale = stale;
                            if stale {
                                // Never carry a partially completed gesture over a gap.
                                view.cancel_touch();
                                view.pointer_feedback = None;
                            }
                            cx.notify();
                        } else if view.show_stats {
                            // Update frame age only while the metrics panel is open.
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
        remote
    }

    fn start(
        &mut self,
        request_initial_control: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stop();
        self.paused = false;
        self.viewing_requested = true;
        self.recovering = false;
        self.recovery_geometry = None;
        self.recovery_wait_ms = 0;
        self.recovery_reason = None;
        self.video_attempt = 0;
        self.auto_control_pending = request_initial_control;
        self.video_stale = true;
        self.pointer_feedback = None;
        self.message = "Abrindo transmissão…".into();
        self.samples.clear();
        self.paint_metrics = Arc::new(PaintMetrics::default());
        self.started = Instant::now();
        self.last_metrics = Instant::now();
        if let Some(old) = self.frame.take() {
            if let Err(error) = window.drop_image(old.image.clone()) {
                eprintln!("TetherDeck: falha ao liberar textura da tela remota: {error}");
            }
        }
        let generation = self.generation;
        let (send, mut receive) = watch::channel(VideoEvent {
            generation,
            attempt: 0,
            state: VideoState::Starting,
        });
        let (cancel, cancelled) = watch::channel(false);
        self.stream_cancel = Some(cancel);
        let session = self.session.clone();
        self.stream = Some(self.runtime.spawn(supervise_video(
            session,
            send,
            cancelled,
            generation,
            self.input_in_flight.clone(),
            self.retiring_stream.take(),
        )));
        self.pump = Some(cx.spawn_in(window, async move |this, cx| {
            while receive.changed().await.is_ok() {
                let event = receive.borrow_and_update().clone();
                if this
                    .update_in(cx, |view, window, cx| {
                        if event.generation != view.generation || event.attempt < view.video_attempt
                        {
                            return;
                        }
                        // watch may skip a transient recovery state while the
                        // foreground is busy. A new attempt must still clear
                        // its predecessor's image and pending pointer gesture.
                        if event.attempt > view.video_attempt
                            && event.attempt > 1
                            && !view.recovering
                        {
                            view.enter_recovery(
                                "Reabrindo o vídeo após interrupção.".into(),
                                0,
                                window,
                            );
                        }
                        view.video_attempt = event.attempt;
                        view.apply(event.state, window, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
        cx.notify();
    }

    fn stop(&mut self) {
        self.cancel_touch();
        self.generation = self.generation.wrapping_add(1).max(1);
        self.viewing_requested = false;
        self.recovering = false;
        self.recovery_wait_ms = 0;
        if let Some(cancel) = self.stream_cancel.take() {
            let _ = cancel.send(true);
        }
        if let Some(stream) = self.stream.take() {
            let previous = self.retiring_stream.take();
            self.retiring_stream = Some(self.runtime.spawn(async move {
                if let Some(previous) = previous {
                    let _ = previous.await;
                }
                let _ = stream.await;
            }));
        }
        self.pump = None;
        // Remember the user's mode across pause/restart. Freshness and
        // geometry gates suspend actual input independently of this preference.
        self.auto_control_pending = false;
        self.pointer_feedback = None;
        self.image_bounds.set(None);
        self.write_present_metrics(true);
    }

    fn apply(&mut self, state: VideoState, window: &mut Window, cx: &mut Context<Self>) {
        match state {
            VideoState::Frame(frame) => {
                if self.recovering {
                    // Restoring a viewer never restores a pointer-down. Only
                    // prior control intent and the identical validated mapping
                    // can be armed, and an earlier input error remains sticky.
                    if frame.received.elapsed() < Duration::from_secs(2) {
                        if !frame
                            .geometry
                            .is_some_and(|mapping| Some(mapping) == self.recovery_geometry)
                            || self.action_error.is_some()
                        {
                            self.control_requested = false;
                            self.auto_control_pending = false;
                        }
                        if frame.geometry.is_some() {
                            self.recovery_count += 1;
                        }
                        self.recovering = false;
                        self.recovery_wait_ms = 0;
                    }
                }
                if !self.fresh() {
                    // The foreground timer may not have run during the gap.
                    // A fresh frame never completes an older pointer gesture.
                    self.cancel_touch();
                    self.pointer_feedback = None;
                }
                if self
                    .frame
                    .as_ref()
                    .is_some_and(|old| old.geometry != frame.geometry || old.size != frame.size)
                {
                    self.control_requested = false;
                    self.auto_control_pending = false;
                    self.cancel_touch();
                }
                if let Some(old) = self.frame.replace(frame) {
                    if let Err(error) = window.drop_image(old.image.clone()) {
                        eprintln!("TetherDeck: falha ao liberar textura da tela remota: {error}");
                    }
                }
                if let Some(mapping) = self.frame.as_ref().and_then(|frame| frame.geometry) {
                    let aspect = mapping.physical[0] as f32 / mapping.physical[1] as f32;
                    if self.chrome_aspect != Some(aspect) {
                        self.chrome_aspect = Some(aspect);
                        match window_chrome::configure(
                            window,
                            aspect,
                            HORIZONTAL_CHROME,
                            VERTICAL_CHROME,
                        ) {
                            Ok(()) => {
                                if !window.is_maximized() {
                                    let viewport = window.viewport_size();
                                    let maximum =
                                        window.display(cx).map(|display| display.bounds().size);
                                    let mut height = f32::from(viewport.height);
                                    let mut width = (height - VERTICAL_CHROME).max(1.) * aspect
                                        + HORIZONTAL_CHROME;
                                    if let Some(maximum) = maximum {
                                        let max_width = (f32::from(maximum.width) - 48.).max(360.);
                                        if width > max_width {
                                            width = max_width;
                                            height = (width - HORIZONTAL_CHROME) / aspect
                                                + VERTICAL_CHROME;
                                        }
                                    }
                                    window.resize(size(px(width), px(height)));
                                }
                            }
                            Err(error) => eprintln!(
                                "TetherDeck: configuração da janela remota indisponível: {error}"
                            ),
                        }
                    }
                }
                self.video_stale = !self.fresh();
                if self.auto_control_pending
                    && self.action_error.is_none()
                    && self.fresh()
                    && self
                        .frame
                        .as_ref()
                        .is_some_and(|frame| frame.geometry.is_some())
                {
                    self.control_requested = true;
                    self.auto_control_pending = false;
                }
                self.message = if self
                    .frame
                    .as_ref()
                    .is_some_and(|frame| frame.geometry.is_none())
                {
                    "Controle suspenso: enquadramento não confirmado. Confira a orientação e retome.".into()
                } else {
                    "Mantenha o mouse pressionado para controlar o toque ao vivo.".into()
                };
                let at = Instant::now();
                self.samples
                    .push_back((at, self.paint_metrics.count.load(Ordering::Relaxed)));
                while self.samples.len() > 2
                    && (self.samples.len() > 241
                        || self
                            .samples
                            .front()
                            .is_some_and(|(at, _)| at.elapsed() > Duration::from_secs(2)))
                {
                    self.samples.pop_front();
                }
                if self.last_metrics.elapsed() >= Duration::from_secs(2) {
                    self.write_present_metrics(false);
                    self.last_metrics = Instant::now();
                }
            }
            VideoState::Ended(message) => {
                self.stream = None;
                self.stream_cancel = None;
                self.viewing_requested = false;
                self.recovering = false;
                self.auto_control_pending = false;
                self.cancel_touch();
                self.message = message;
                self.write_present_metrics(true);
            }
            VideoState::Reconnecting { reason, delay_ms } => {
                self.enter_recovery(reason, delay_ms, window);
                self.write_present_metrics(false);
            }
            VideoState::Starting => {
                if self.video_attempt > 1 {
                    self.message =
                        format!("Reconectando vídeo · tentativa {}…", self.video_attempt);
                    self.recovery_wait_ms = 0;
                }
            }
        }
        cx.notify();
    }

    fn enter_recovery(&mut self, reason: String, delay_ms: u64, window: &mut Window) {
        self.cancel_touch();
        if let Some(mapping) = self.frame.as_ref().and_then(|frame| frame.geometry) {
            self.recovery_geometry = Some(mapping);
        }
        if self.action_error.is_some() {
            self.control_requested = false;
            self.auto_control_pending = false;
        }
        self.recovering = true;
        self.video_stale = true;
        self.recovery_attempts = self.video_attempt;
        self.recovery_wait_ms = delay_ms;
        self.recovery_reason = Some(reason);
        self.message = format!(
            "Reconectando vídeo automaticamente · tentativa {} em {:.1} s. Nenhum gesto será repetido.",
            self.video_attempt + 1,
            delay_ms as f64 / 1000.
        );
        self.image_bounds.set(None);
        self.samples.clear();
        if let Some(old) = self.frame.take() {
            let _ = window.drop_image(old.image.clone());
        }
    }

    fn fresh(&self) -> bool {
        self.frame
            .as_ref()
            .is_some_and(|frame| frame.received.elapsed() < Duration::from_secs(2))
    }
    fn control_ready(&self) -> bool {
        self.viewing_requested()
            && !self.recovering()
            && self.fresh()
            && self
                .frame
                .as_ref()
                .is_some_and(|frame| frame.geometry.is_some())
    }

    fn viewing_requested(&self) -> bool {
        self.viewing_requested
    }
    fn recovering(&self) -> bool {
        self.recovering
    }
    fn allowed(&self) -> bool {
        self.control_requested && self.control_ready() && !self.busy
    }
    fn hardware_buttons_allowed(&self) -> bool {
        // A deliberate Home/Lock/Volume press does not use image coordinates.
        // Permit waking a locked screen while an existing viewer is stale,
        // but never race stream setup/recovery or an uncertain input result.
        self.control_requested
            && self.viewing_requested()
            && !self.recovering()
            && self.frame.is_some()
            && self.action_error.is_none()
            && !self.busy
    }
    fn fps(&self) -> f64 {
        match (self.samples.front(), self.samples.back()) {
            (Some((first, a)), Some((last, b))) if last > first => {
                b.saturating_sub(*a) as f64 / last.duration_since(*first).as_secs_f64()
            }
            _ => 0.,
        }
    }

    fn write_present_metrics(&self, stopped: bool) {
        let Some(path) = std::env::var_os("TETHERDECK_PRESENT_METRICS").filter(|p| !p.is_empty())
        else {
            return;
        };
        let value = serde_json::json!({ "backend":"gpui", "state":if stopped {"stopped"} else if self.recovering {"reconnecting"} else {"streaming"}, "paint_count":self.paint_metrics.count.load(Ordering::Relaxed), "decoded_sequence":self.frame.as_ref().map(|f| f.sequence).unwrap_or(0), "elapsed_seconds":self.started.elapsed().as_secs_f64(), "paint_fps_recent":self.fps(), "recovery_attempts":self.recovery_attempts,"recovery_count":self.recovery_count,"next_retry_ms":self.recovery_wait_ms,"video_generation":self.generation, "measurement":"GPUI paint calls for distinct video frames; not GPU display fences or end-to-end latency" });
        self.runtime.spawn(async move {
            let _ = tokio::fs::write(path, value.to_string()).await;
        });
    }

    fn command(&mut self, action: HidAction, window: &mut Window, cx: &mut Context<Self>) {
        let permitted = if matches!(&action, HidAction::DeviceButton { .. }) {
            self.hardware_buttons_allowed()
        } else {
            self.allowed()
        };
        if !permitted {
            return;
        }
        self.busy = true;
        self.action_error = None;
        self.drag = None;
        let action_kind = match &action {
            HidAction::Tap { .. } => "tap",
            HidAction::Press { .. } => "press",
            HidAction::Drag { .. } => "drag",
            HidAction::DeviceButton { .. } => "button",
            HidAction::TypeText { .. } => "text",
        };
        let session = self.session.clone();
        let (send, receive) = oneshot::channel();
        // The OS worker owns Runtime outside the async future. Closing both
        // windows cannot destroy Tokio while this command is completing its
        // release/end sequence. The UI never joins this thread.
        let runtime = self.runtime.clone();
        let input_guard = InputInFlight::new(self.input_in_flight.clone());
        if let Err(error) = std::thread::Builder::new()
            .name("tetherdeck-input".into())
            .spawn(move || {
                let _input_guard = input_guard;
                runtime.block_on(async move {
            let started = Instant::now();
            let result = session.client.action(&session.device_id, &action).await.map_err(|error| error.to_string());
            let ms = started.elapsed().as_millis() as u64;
            let _ = send.send((result, ms));
            if let Some(path) = std::env::var_os("TETHERDECK_INPUT_METRICS").filter(|p| !p.is_empty()) {
                let value = serde_json::json!({"backend":"gpui","action_kind":action_kind,"command_response_ms":ms,"measurement":"API request to response; not end-to-end interaction or video latency"});
                let _ = tokio::fs::write(path, value.to_string()).await;
            }
            drop(session);
                });
                // Last Runtime Arc, if any, drops here outside an async context.
            })
        {
            // Failed spawn drops the oneshot sender; the normal UI failure path
            // records an unavailable result and disables interaction.
            eprintln!("TetherDeck: não foi possível iniciar trabalhador de entrada: {error}");
        }
        self.command_task = Some(cx.spawn_in(window, async move |this, cx| {
            let response = receive.await;
            let _ = this.update_in(cx, |view, _, cx| {
                view.busy = false;
                match response {
                    Ok((Ok(()), ms)) => { view.command_ms = Some(ms); view.message = "Comando enviado. Confira o resultado na tela.".into(); }
                    Ok((Err(error), ms)) => { view.command_ms = Some(ms); view.action_error = Some(error); view.control_requested = false; view.auto_control_pending = false; }
                    Err(_) => { view.action_error = Some("Resposta do comando indisponível; confira o aparelho antes de repetir.".into()); view.control_requested = false; view.auto_control_pending = false; }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn copy_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(frame) = self.frame.clone() else {
            return;
        };
        if self.busy {
            return;
        }
        self.busy = true;
        let work = self
            .runtime
            .spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
                let [width, height] = frame.geometry.map(|g| g.crop).unwrap_or(frame.size);
                let source = frame
                    .image
                    .as_bytes(0)
                    .ok_or_else(|| anyhow::anyhow!("Quadro indisponível"))?;
                let mut rgba = Vec::with_capacity(width * height * 4);
                for row in source.chunks_exact(frame.size[0] * 4).take(height) {
                    rgba.extend_from_slice(&row[..width * 4]);
                }
                for pixel in rgba.chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                }
                let mut png = Vec::new();
                image::codecs::png::PngEncoder::new(&mut png).write_image(
                    &rgba,
                    width as u32,
                    height as u32,
                    image::ExtendedColorType::Rgba8,
                )?;
                Ok(png)
            });
        self.command_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |view, _, cx| {
                view.busy = false;
                match result {
                    Ok(Ok(bytes)) => {
                        cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(
                            ImageFormat::Png,
                            bytes,
                        )));
                        view.message = "Quadro copiado para a área de transferência.".into();
                    }
                    _ => view.message = "Não foi possível preparar a captura.".into(),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn pointer_point(&self, point: Point<Pixels>) -> Option<(u16, u16)> {
        let bounds = self.image_bounds.get()?;
        let frame = self.frame.as_ref()?;
        let geometry = frame.geometry?;
        if bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
            return None;
        }
        let x = f32::from(point.x - bounds.origin.x) / f32::from(bounds.size.width);
        let y = f32::from(point.y - bounds.origin.y) / f32::from(bounds.size.height);
        Some((
            (x * geometry.physical[0] as f32)
                .floor()
                .clamp(0., geometry.physical[0].saturating_sub(1) as f32) as u16,
            (y * geometry.physical[1] as f32)
                .floor()
                .clamp(0., geometry.physical[1].saturating_sub(1) as f32) as u16,
        ))
    }

    fn cancel_touch(&mut self) {
        self.drag = None;
        if let Some(intent) = &self.touch_intent {
            let point = match *intent.borrow() {
                TouchIntent::Held(point) | TouchIntent::End(point) | TouchIntent::Cancel(point) => {
                    point
                }
            };
            intent.send_replace(TouchIntent::Cancel(point));
        }
        self.pointer_feedback = None;
    }

    fn pointer_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window_chrome::is_resize_area(window, event.position) {
            self.cancel_touch();
            return;
        }
        if !self.allowed()
            || !self
                .image_bounds
                .get()
                .is_some_and(|bounds| bounds.contains(&event.position))
        {
            return;
        }
        let Some(point) = self.pointer_point(event.position) else {
            return;
        };
        let Some(geometry) = self.frame.as_ref().and_then(|frame| frame.geometry) else {
            return;
        };
        let (send_intent, receive_intent) = watch::channel(TouchIntent::Held(point));
        self.touch_intent = Some(send_intent.clone());
        let (send_result, receive_result) = oneshot::channel();
        let session = self.session.clone();
        let runtime = self.runtime.clone();
        let gesture_id = uuid::Uuid::new_v4().to_string();
        let input_guard = InputInFlight::new(self.input_in_flight.clone());
        self.busy = true;
        self.action_error = None;
        if let Err(error) = std::thread::Builder::new()
            .name("tetherdeck-touch".into())
            .spawn(move || {
                let _input_guard = input_guard;
                runtime.block_on(async move {
                    let result = live_touch(&session, receive_intent, gesture_id, point).await;
                    let _ = send_result.send(result);
                    drop(session);
                });
                // Keep Runtime alive until release completes, outside async Drop.
            })
        {
            eprintln!("TetherDeck: não foi possível iniciar trabalhador de toque: {error}");
        }
        self.touch_reply = Some(cx.spawn_in(window, async move |this, cx| {
            let result = receive_result.await;
            let _ = this.update_in(cx, |view, _, cx| {
                view.busy = false;
                view.drag = None;
                view.touch_intent = None;
                match result {
                    Ok((Ok(()), ms)) => {
                        view.command_ms = Some(ms);
                    }
                    Ok((Err(error), ms)) => {
                        view.command_ms = Some(ms);
                        view.action_error = Some(error);
                        view.control_requested = false;
                        view.auto_control_pending = false;
                    }
                    Err(_) => {
                        view.action_error = Some(
                            "Resposta do toque indisponível; confira o aparelho antes de repetir."
                                .into(),
                        );
                        view.control_requested = false;
                        view.auto_control_pending = false;
                    }
                }
                cx.notify();
            });
        }));
        self.pointer_feedback = Some((event.position, Instant::now()));
        cx.notify();
        self.drag = Some(Drag {
            point,
            geometry,
            intent: send_intent,
        });
    }

    fn pointer_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.drag.is_none() {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left)
            || !self.control_requested
            || !self.control_ready()
            || self.frame.as_ref().and_then(|frame| frame.geometry)
                != self.drag.as_ref().map(|drag| drag.geometry)
        {
            self.cancel_touch();
            cx.notify();
            return;
        }
        let Some(point) = self.pointer_point(event.position) else {
            self.cancel_touch();
            return;
        };
        if let Some(drag) = &mut self.drag {
            drag.point = point;
            drag.intent.send_replace(TouchIntent::Held(point));
        }
        self.pointer_feedback = Some((event.position, Instant::now()));
        cx.notify();
    }

    fn pointer_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if !self.control_requested
            || !self.control_ready()
            || self.frame.as_ref().and_then(|frame| frame.geometry) != Some(drag.geometry)
        {
            drag.intent.send_replace(TouchIntent::Cancel(drag.point));
            self.pointer_feedback = None;
            cx.notify();
            return;
        }
        let Some(end) = self.pointer_point(event.position) else {
            drag.intent.send_replace(TouchIntent::Cancel(drag.point));
            return;
        };
        drag.intent.send_replace(TouchIntent::End(end));
        self.pointer_feedback = Some((event.position, Instant::now()));
        cx.notify();
    }
}

impl Render for Remote {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let running = self.viewing_requested();
        // Opting out is possible even while the video is temporarily stale.
        let toggle_allowed =
            (!self.busy || self.drag.is_some()) && (self.control_requested || self.control_ready());
        let canvas_frame = self.frame.clone();
        let pointer_feedback = self.pointer_feedback;
        let pointer_held = self.drag.is_some();
        let reduce_motion = cx.reduce_motion();
        let controlling = self.control_requested && self.control_ready();
        let control_suspended = self.control_requested && !self.control_ready();
        let content = self
            .frame
            .as_ref()
            .map(|frame| frame.geometry.map(|g| g.physical).unwrap_or(frame.size));
        let viewport = window.viewport_size();
        let aspect = content
            .map(|size| size[0] as f32 / size[1].max(1) as f32)
            .unwrap_or(9. / 19.5);
        let max_phone_width = (f32::from(viewport.width) - 76.).max(1.);
        let max_phone_height = (f32::from(viewport.height) - 64.).max(1.);
        let phone_width = (((max_phone_height - 12.).max(1.) * aspect) + 12.).min(max_phone_width);
        let phone_height = ((phone_width - 12.).max(1.) / aspect) + 12.;
        let group_width = phone_width + 60.;

        let bounds_store = self.image_bounds.clone();
        let canvas_size = self.canvas_size.clone();
        let paint_metrics = self.paint_metrics.clone();
        let screen = div()
            .id("remote-screen")
            .w(px(phone_width))
            .flex_shrink_0()
            .min_w_0()
            .h(px(phone_height))
            .aria_label(tr("Tela remota do iPhone"))
            .aria_description(tr("Clique para tocar. Mantenha o mouse pressionado e mova para arrastar ao vivo; solte para levantar o dedo."))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::pointer_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::pointer_up))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| {
                    view.cancel_touch();
                    cx.notify();
                }),
            )
            .child(
                canvas(
                    move |bounds, window, _| {
                        window_chrome::update_resize_bounds(window, bounds);
                        let previous = canvas_size.replace(Some(bounds.size));
                        if previous.is_none_or(|size| (f32::from(size.height) - f32::from(bounds.size.height)).abs() > 0.5) {
                            // One layout correction after resize, including when
                            // paused. This does not run as a decorative animation.
                            window.request_animation_frame();
                        }
                        let Some(frame) = canvas_frame else {
                            bounds_store.set(None);
                            return None;
                        };
                        let content = frame.geometry.map(|g| g.physical).unwrap_or(frame.size);
                        // A 6 px generic bezel follows the actual image rectangle.
                        let scale = ((f32::from(bounds.size.width) - 12.).max(1.) / content[0].max(1) as f32)
                            .min((f32::from(bounds.size.height) - 12.).max(1.) / content[1].max(1) as f32);
                        let visible_size =
                            size(px(content[0] as f32 * scale), px(content[1] as f32 * scale));
                        let visible = Bounds {
                            origin: point(
                                bounds.origin.x + (bounds.size.width - visible_size.width) / 2.,
                                bounds.origin.y + (bounds.size.height - visible_size.height) / 2.,
                            ),
                            size: visible_size,
                        };
                        bounds_store.set(Some(visible));
                        let uv = frame.geometry.map(|g| g.uv).unwrap_or([1., 1.]);
                        let full = Bounds {
                            origin: visible.origin,
                            size: size(visible.size.width / uv[0], visible.size.height / uv[1]),
                        };
                        Some((frame, visible, full))
                    },
                    move |_, prepared, window, _| {
                        if let Some((frame, visible, full)) = prepared {
                            let bezel = Bounds { origin: point(visible.origin.x - px(6.), visible.origin.y - px(6.)), size: size(visible.size.width + px(12.), visible.size.height + px(12.)) };
                            let radius = (f32::from(visible.size.width) * 0.065).clamp(12., 26.);
                            window.paint_quad(quad(bezel, px(radius), rgb(0x090b10), px(1.),
                                    if controlling { theme::accent().opacity(0.75) } else { theme::border() }, BorderStyle::default()));
                            if window
                                .paint_image(
                                    visible,
                                    full,
                                    Corners::all(px((radius - 5.).max(0.))),
                                    frame.image.clone(),
                                    0,
                                    false,
                                )
                                .is_ok()
                                && paint_metrics
                                    .sequence
                                    .swap(frame.sequence, Ordering::Relaxed)
                                    != frame.sequence
                            {
                                paint_metrics.count.fetch_add(1, Ordering::Relaxed);
                            }
                            if let Some((position, at)) = pointer_feedback {
                                let elapsed = at.elapsed().as_secs_f32();
                                if (pointer_held || elapsed < 0.16) && visible.contains(&position) {
                                    let opacity = if pointer_held || reduce_motion { 1.0 } else { 1.0 - elapsed / 0.16 };
                                    let ring = Bounds::new(point(position.x - px(9.), position.y - px(9.)), size(px(18.), px(18.)));
                                    window.paint_quad(quad(ring, px(9.), theme::accent().opacity(0.12), px(2.), theme::accent().opacity(opacity), BorderStyle::default()));
                                    if !pointer_held && !reduce_motion { window.request_animation_frame(); }
                                }
                            }
                        }
                    },
                )
                .size_full(),
            );
        let mut toolbar = div()
            .id("remote-toolbar")
            .flex().flex_col().items_center().gap(px(6.))
            .w(px(52.)).max_h(px(phone_height)).flex_shrink_0()
            .p(px(7.)).bg(theme::surface()).border_1()
            .border_color(theme::border()).rounded(px(8.)).overflow_y_scroll();
        for (index, (id, icon, action_name, button)) in [
            ("home", IconName::House, "Ir à tela inicial do iPhone", DeviceButton::Home),
            ("apps", IconName::AppWindow, "Abrir alternador de aplicativos do iPhone", DeviceButton::AppSwitcher),
            ("volume-up", IconName::Volume2, "Aumentar volume do iPhone", DeviceButton::VolumeUp),
            ("volume-down", IconName::Volume1, "Diminuir volume do iPhone", DeviceButton::VolumeDown),
            ("lock", IconName::Lock, "Bloquear ou acender a tela do iPhone", DeviceButton::Lock),
        ].into_iter().enumerate() {
            if index == 2 || index == 4 {
                toolbar = toolbar.child(div().w_full().h(px(1.)).my(px(2.)).bg(theme::border()));
            }
            toolbar = toolbar.child(
                control_button(id).icon(crate::icons::icon(icon)).w(px(36.)).h(px(36.)).min_h(px(36.))
                    .tooltip(tr(action_name)).accessibility_label(tr(action_name))
                    .disabled(!self.hardware_buttons_allowed())
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.command(HidAction::DeviceButton { button }, window, cx);
                        // Locking is an explicit device action, not a change
                        // of viewing preference. Stale frames still suspend
                        // input and cancel contact until fresh video returns.
                    })),
            );
        }
        toolbar = toolbar
            .child(div().w_full().h(px(1.)).my(px(2.)).bg(theme::border()))
            .child(control_button("capture").icon(crate::icons::icon(IconName::Camera))
                .w(px(36.)).h(px(36.)).min_h(px(36.))
                .tooltip(tr("Copiar quadro exibido")).accessibility_label(tr("Copiar captura da tela do iPhone"))
                .disabled(self.frame.is_none() || self.busy)
                .on_click(cx.listener(|view, _, window, cx| view.copy_frame(window, cx))));
        let status = if running && self.fresh() {
            format!("{} · {:.1} FPS", tr("Ao vivo"), self.fps())
        } else if self.recovering() {
            "Reconectando automaticamente…".into()
        } else if self.paused {
            "Pausado".into()
        } else if !running {
            "Transmissão encerrada".into()
        } else {
            "Aguardando vídeo".into()
        };
        let status = tr(&status);
        let status_message = self
            .action_error
            .clone()
            .unwrap_or_else(|| if control_suspended {
                "Controle suspenso enquanto o vídeo atualiza. Retomará quando a imagem estiver recente.".into()
            } else {
                self.message.clone()
            });
        let status_message = tr(&status_message);
        let status_tooltip = status_message.clone();
        let maximize_icon = if window.is_maximized() {
            crate::icons::CaptionIcon::Restore
        } else {
            crate::icons::CaptionIcon::Maximize
        };
        let maximize_label = if window.is_maximized() {
            "Restaurar tamanho da janela"
        } else {
            "Maximizar janela"
        };
        let control_label = if control_suspended {
            "Controle temporariamente suspenso: mudar para somente visualizar"
        } else if controlling {
            "Controle ativo: mudar para somente visualizar"
        } else {
            "Somente visualizar: ativar controle do iPhone"
        };
        let header = div()
            .id("remote-chrome")
            .w_full()
            .h(px(40.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(4.))
            .bg(theme::surface())
            .border_1()
            .border_color(theme::border())
            .rounded(px(10.))
            .child(
                control_button("pause")
                    .icon(crate::icons::icon(if running {
                        IconName::Pause
                    } else {
                        IconName::Play
                    }))
                    .w(px(32.))
                    .h(px(32.))
                    .rounded(px(6.))
                    .tooltip(tr(if running {
                        "Pausar transmissão"
                    } else {
                        "Retomar transmissão"
                    }))
                    .accessibility_label(tr(if running {
                        "Pausar transmissão da tela"
                    } else {
                        "Retomar transmissão da tela"
                    }))
                    .disabled(self.busy && self.drag.is_none())
                    .on_click(cx.listener(|view, _, window, cx| {
                        if view.viewing_requested() {
                            view.stop();
                            view.paused = true;
                            view.message = "Transmissão pausada.".into();
                            cx.notify();
                        } else {
                            view.start(false, window, cx);
                        }
                    })),
            )
            .child(
                control_button("interact")
                    .when(self.control_requested, |button| {
                        button.bg(theme::accent().opacity(0.20)).border_color(theme::accent().opacity(0.45))
                    })
                    .icon(crate::icons::icon(IconName::MousePointer2))
                    .w(px(32.))
                    .h(px(32.))
                    .rounded(px(8.))
                    .accessibility_label(tr(control_label))
                    .tooltip(tr(control_label))
                    .selected(self.control_requested)
                    .disabled(!toggle_allowed)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.cancel_touch();
                        view.control_requested = !view.control_requested;
                        view.auto_control_pending = false;
                        view.action_error = None;
                        cx.notify();
                    })),
            )
            .child(
                control_button("statistics")
                    .when(self.show_stats, |button| {
                        button.bg(theme::accent().opacity(0.20)).border_color(theme::accent().opacity(0.45))
                    })
                    .icon(crate::icons::icon(IconName::ChartNoAxesCombined))
                    .w(px(32.))
                    .h(px(32.))
                    .rounded(px(8.))
                    .accessibility_label(tr(if self.show_stats {
                        "Ocultar estatísticas da transmissão"
                    } else {
                        "Mostrar estatísticas da transmissão"
                    }))
                    .tooltip(tr("Estatísticas da transmissão"))
                    .selected(self.show_stats)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.show_stats = !view.show_stats;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("remote-title-drag")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .cursor(CursorStyle::OpenHand)
                    .window_control_area(WindowControlArea::Drag)
                    .aria_label(tr("Arrastar janela da tela remota"))
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(status_tooltip.clone())
                            .build(window, cx)
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _: &MouseDownEvent, _, _| {
                            view.cancel_touch();
                            // GPUI's native caption hit test owns drag and
                            // double-click. Reposting NC mouse-down here would
                            // deliver a second GPUI input event recursively.
                        }),
                    )
                    .child(div().size(px(5.)).flex_shrink_0().rounded_full().bg(
                        if running && self.fresh() {
                            theme::success()
                        } else {
                            theme::warning()
                        },
                    ))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(theme::muted())
                            .child(
                                if self.recovering() { tr("Reconectando…") } else { crate::desktop::device_model(self.session.product_type.as_deref()).to_owned() },
                            ),
                    ),
            )

            .child(
                control_button("minimize")
                    .child(crate::icons::caption_icon(crate::icons::CaptionIcon::Minimize))
                    .w(px(32.))
                    .h(px(32.))
                    .rounded(px(6.))
                    .tooltip(tr("Minimizar janela"))
                    .accessibility_label(tr("Minimizar janela da tela remota"))
                    .on_click(cx.listener(|view, _, window, _| {
                        view.cancel_touch();
                        if let Err(error) = window_chrome::minimize(window) {
                            eprintln!("TetherDeck: minimizar janela: {error}");
                        }
                    })),
            )
            .child(
                control_button("maximize")
                    .child(crate::icons::caption_icon(maximize_icon))
                    .w(px(32.))
                    .h(px(32.))
                    .rounded(px(6.))
                    .tooltip(tr(maximize_label))
                    .accessibility_label(tr(maximize_label))
                    .on_click(cx.listener(|view, _, window, _| {
                        view.cancel_touch();
                        if let Err(error) = window_chrome::maximize_restore(window) {
                            eprintln!("TetherDeck: maximizar/restaurar janela: {error}");
                        }
                    })),
            )
            .child(
                control_button("close")
                    .hover(|style| style.bg(theme::error().opacity(0.20)).text_color(theme::text()).border_color(theme::error().opacity(0.5)))
                    .child(crate::icons::caption_icon(crate::icons::CaptionIcon::Close))
                    .w(px(32.))
                    .h(px(32.))
                    .rounded(px(6.))
                    .tooltip(tr("Fechar tela remota"))
                    .accessibility_label(tr("Fechar janela da tela remota"))
                    .on_click(cx.listener(|view, _, window, _| {
                        view.cancel_touch();
                        if let Err(error) = window_chrome::close(window) {
                            eprintln!("TetherDeck: fechar janela: {error}");
                        }
                    })),
            );

        let mut group = div()
            .relative()
            .w(px(group_width))
            .h(px(phone_height + 48.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(header)
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .h(px(phone_height))
                    .child(screen)
                    .child(toolbar),
            );
        if self.frame.is_none() {
            group = group.child(
                div()
                    .absolute()
                    .top(px(80.))
                    .left(px(16.))
                    .w(px((phone_width - 32.).max(1.)))
                    .p(px(12.))
                    .rounded(px(10.))
                    .bg(theme::surface())
                    .text_color(theme::muted())
                    .text_size(px(12.))
                    .child(status.clone()),
            );
        }
        let show_status_message = self.action_error.is_some()
            || (control_suspended && running)
            || (!running && !self.paused && self.frame.is_some());
        if show_status_message && !self.show_stats {
            group = group.child(
                div()
                    .id("remote-message")
                    .absolute()
                    .bottom(px(12.))
                    .left(px(12.))
                    .w(px((phone_width - 24.).max(1.)))
                    .p(px(8.))
                    .rounded(px(8.))
                    .bg(theme::surface())
                    .border_1()
                    .border_color(theme::border())
                    .text_size(px(11.))
                    .text_color(if self.action_error.is_some() { theme::error() } else { theme::muted() })
                    .child(status_message.clone()),
            );
        }
        if self.show_stats {
            let fps = if running && self.fresh() {
                self.fps()
            } else {
                0.
            };
            let interval = if fps > 0. {
                format!("{:.1} ms", 1000. / fps)
            } else {
                "—".into()
            };
            let age = self
                .frame
                .as_ref()
                .map(|frame| format!("{} ms", frame.received.elapsed().as_millis()))
                .unwrap_or_else(|| "—".into());
            let origin = self
                .frame
                .as_ref()
                .and_then(|frame| frame.geometry)
                .map(|g| format!("{} × {}", g.physical[0], g.physical[1]))
                .unwrap_or_else(|| tr("Não confirmada"));
            let preview = self
                .frame
                .as_ref()
                .map(|frame| format!("{} × {}", frame.size[0], frame.size[1]))
                .unwrap_or_else(|| "—".into());
            let response = self
                .command_ms
                .map(|ms| {
                    if ms == 0 {
                        "<1 ms".into()
                    } else {
                        format!("{ms} ms")
                    }
                })
                .unwrap_or_else(|| "—".into());
            let mut stats = div()
                .id("remote-statistics-panel")
                .absolute()
                .top(px(60.))
                .left(px(12.))
                .w(px((phone_width - 24.).clamp(180., 264.)))
                .flex()
                .flex_col()
                .gap(px(8.))
                .p(px(12.))
                .rounded(px(10.))
                .bg(theme::surface())
                .border_1()
                .border_color(theme::border())
                .text_size(px(11.))
                .text_color(theme::text())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(tr("Transmissão")))
                        .child(div().text_color(theme::muted()).child(status)),
                )
                .child(div().h(px(1.)).bg(theme::border()));
            for (label, value) in [
                ("Pintura GPUI", format!("{fps:.1} FPS")),
                ("Intervalo médio", interval),
                ("Idade local do quadro", age),
                ("Resposta do comando", response),
                ("Tela de origem", origin),
                ("Preview", preview),
                ("Recuperações", format!("{} / {} {}", self.recovery_count, self.recovery_attempts, tr("tentativas"))),
            ] {
                stats = stats.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(8.))
                        .child(div().text_color(theme::muted()).child(tr(label)))
                        .child(div().child(value)),
                );
            }
            stats = stats.child(div().flex().gap(px(6.))
                .child(div().flex_1().p(px(6.)).rounded(px(6.)).bg(if fps >= 30. { theme::success().opacity(0.12) } else { theme::button_surface() }).child(tr("30 FPS")))
                .child(div().flex_1().p(px(6.)).rounded(px(6.)).bg(if fps >= 60. { theme::success().opacity(0.12) } else { theme::button_surface() }).child(tr("60 FPS"))))
                .child(div().text_size(px(10.)).text_color(theme::muted())
                    .child(tr("FPS de quadros distintos pintados pelo GPUI. Intervalo = 1000/FPS. Resposta é da API; não mede latência ponta a ponta.")));
            if show_status_message {
                stats = stats.child(
                    div()
                        .text_size(px(10.))
                        .text_color(theme::warning())
                        .child(status_message),
                );
            }
            if self.recovering() {
                let retry = if self.recovery_wait_ms == 0 {
                    tr("Abrindo um novo fluxo…")
                } else {
                    format!("{}: {:.1} s", tr("Espera programada"), self.recovery_wait_ms as f64 / 1000.)
                };
                stats = stats.child(div().text_color(theme::warning()).child(retry));
            }
            if let Some(reason) = &self.recovery_reason {
                stats = stats.child(div().text_size(px(10.)).text_color(theme::muted()).child(format!("{}: {reason}", tr("Última interrupção"))));
            }
            group = group.child(stats);
        }
        let surface = div()
            .id("remote-transparent-surface")
            .relative()
            .size_full()
            .on_mouse_move(cx.listener(Self::pointer_move))
            .on_mouse_exit(cx.listener(|view, _, _, cx| {
                view.cancel_touch();
                cx.notify();
            }))
            .flex()
            .items_center()
            .justify_center()
            .p(px(8.))
            .text_color(theme::text())
            .text_size(px(13.))
            .child(group);
        surface
    }
}
