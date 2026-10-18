//! Independent remote window. Video uses a latest-frame mailbox, never the command worker.
use super::icons::Icon as ToolIcon;
use super::*;
use super::{icons, theme};
use rust_ios_device_api::DeviceButton;

#[derive(Clone)]
enum State {
    Connecting,
    Frame {
        image: Arc<egui::ColorImage>,
        sequence: u64,
        received: Instant,
        source_size: [usize; 2],
    },
    Ended(String),
}

pub(super) struct Feed {
    id: String,
    origin: String,
    state: tokio::sync::watch::Receiver<State>,
    worker: tokio::task::JoinHandle<()>,
    sequence: u64,
    samples: VecDeque<(Instant, u64)>,
    uploaded_samples: VecDeque<(Instant, u64)>,
    upload_count: u64,
    started: Instant,
    last_metrics: Instant,
    present_reporter: Option<tokio::sync::watch::Sender<serde_json::Value>>,
}
impl Drop for Feed {
    fn drop(&mut self) {
        self.report_metrics(true);
        self.worker.abort();
    }
}
impl Feed {
    fn report_metrics(&self, stopped: bool) {
        let Some(reporter) = &self.present_reporter else {
            return;
        };
        let decoded_sequence = match &*self.state.borrow() {
            State::Frame { sequence, .. } => *sequence,
            _ => self.sequence,
        };
        reporter.send_replace(serde_json::json!({
            "state": if stopped { "stopped" } else { "streaming" },
            "upload_count": self.upload_count,
            "decoded_sequence": decoded_sequence,
            "elapsed_seconds": self.started.elapsed().as_secs_f64(),
            "uploaded_fps_recent": sample_fps(&self.uploaded_samples),
            "decoded_fps_recent": sample_fps(&self.samples),
            "measurement": "egui texture submissions; not GPU display fences or end-to-end video latency",
        }));
    }
}

fn present_reporter(
    runtime: &tokio::runtime::Runtime,
) -> Option<tokio::sync::watch::Sender<serde_json::Value>> {
    let path = std::env::var_os("TETHERDECK_PRESENT_METRICS")?;
    if path.is_empty() {
        return None;
    }
    let (sender, mut receiver) = tokio::sync::watch::channel(serde_json::Value::Null);
    // One coalescing mailbox and a sequential background writer: metrics never
    // block the GUI or accumulate writes when storage is slow. Dropping Feed
    // publishes a final snapshot before the sender closes.
    runtime.spawn(async move {
        while receiver.changed().await.is_ok() {
            let value = receiver.borrow_and_update().clone();
            let _ = tokio::fs::write(&path, value.to_string()).await;
        }
    });
    Some(sender)
}

fn record_sample(samples: &mut VecDeque<(Instant, u64)>, at: Instant, count: u64) {
    samples.push_back((at, count));
    while samples.len() > 2
        && (samples.len() > 241
            || samples
                .front()
                .is_some_and(|sample| at.duration_since(sample.0) > Duration::from_secs(2)))
    {
        samples.pop_front();
    }
}

fn sample_fps(samples: &VecDeque<(Instant, u64)>) -> f64 {
    match (samples.front(), samples.back()) {
        (Some(first), Some(last)) if last.0 > first.0 => {
            last.1.saturating_sub(first.1) as f64 / last.0.duration_since(first.0).as_secs_f64()
        }
        _ => 0.0,
    }
}

/// A verified top-left content rectangle. No CPU image copy is needed to render it.
#[derive(Clone, Copy, PartialEq)]
struct FrameGeometry {
    content_size: [usize; 2],
    uv: egui::Rect,
}

fn frame_geometry(image: &egui::ColorImage, source: [usize; 2]) -> Option<FrameGeometry> {
    let decoded = image.size;
    if source
        .iter()
        .any(|&axis| axis == 0 || axis > u16::MAX as usize)
        || decoded.iter().any(|&axis| axis == 0)
        || image.pixels.len() != decoded[0].checked_mul(decoded[1])?
    {
        return None;
    }
    if decoded != source {
        for axis in 0..2 {
            let physical = source[axis];
            let padded = decoded[axis];
            // Some encoders align one axis to 16 and the other to 64. An
            // unchanged axis needs no alignment inference. No model constants.
            if padded < physical
                || padded - physical >= 64
                || (padded != physical
                    && ![16usize, 64]
                        .iter()
                        .any(|&alignment| physical.div_ceil(alignment) * alignment == padded))
            {
                return None;
            }
        }
        // Sample every padding-depth pixel at 33 evenly spaced positions along
        // the edge, including both endpoints. At most 4,158 pixels per frame.
        // This validates only the small top-left alignment padding established
        // by the source dimensions; it never guesses a crop from image content.
        let dark = |x: usize, y: usize| {
            let pixel = image.pixels[y * decoded[0] + x].to_array();
            pixel[3] >= 250 && pixel[..3].iter().all(|&channel| channel <= 64)
        };
        for x in source[0]..decoded[0] {
            for sample in 0..=32 {
                if !dark(x, sample * (decoded[1] - 1) / 32) {
                    return None;
                }
            }
        }
        for y in source[1]..decoded[1] {
            for sample in 0..=32 {
                if !dark(sample * (decoded[0] - 1) / 32, y) {
                    return None;
                }
            }
        }
    }
    Some(FrameGeometry {
        content_size: source,
        uv: egui::Rect::from_min_max(
            egui::Pos2::ZERO,
            egui::pos2(
                source[0] as f32 / decoded[0] as f32,
                source[1] as f32 / decoded[1] as f32,
            ),
        ),
    })
}

/// State belongs to the session, not the selected workbench page.
#[derive(Default)]
pub(super) struct RemoteWindow {
    open: bool,
    paused: bool,
    target: Option<(String, String)>,
    frame: Option<Arc<egui::ColorImage>>,
    command_started: Option<Instant>,
    command_ms: Option<u64>,
    fullscreen: bool,
    source_size: [usize; 2],
    geometry: Option<FrameGeometry>,
}

impl Studio {
    pub(super) fn open_remote_window(&mut self, ctx: &egui::Context) {
        if !self.api_confirmed || self.selected.is_empty() {
            return;
        }
        let already_open = self.remote_window.open;
        self.remote_window.open = true;
        self.remote_window.target = Some((self.selected.clone(), self.origin.clone()));
        if !already_open {
            self.remote_window.paused = false;
            self.start_live(ctx);
        }
        ctx.send_viewport_cmd_to(remote_viewport_id(), egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }

    pub(super) fn close_remote_window(&mut self, ctx: &egui::Context) {
        self.remote_window.open = false;
        self.remote_window.paused = false;
        self.remote_window.target = None;
        self.stop_live();
        ctx.send_viewport_cmd_to(remote_viewport_id(), egui::ViewportCommand::Close);
    }

    pub(super) fn remote_command_finished(&mut self) {
        if let Some(started) = self.remote_window.command_started.take() {
            let command_response_ms = started.elapsed().as_millis() as u64;
            self.remote_window.command_ms = Some(command_response_ms);
            if let Some(path) = std::env::var_os("TETHERDECK_INPUT_METRICS").filter(|path| !path.is_empty()) {
                let finished_at_unix_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
                let metrics = serde_json::json!({
                    "command_response_ms": command_response_ms,
                    "finished_at_unix_ms": finished_at_unix_ms,
                    "measurement": "GUI dispatch to API response; not end-to-end interaction or video latency",
                });
                self.runtime.spawn(async move {
                    let _ = tokio::fs::write(path, metrics.to_string()).await;
                });
            }
        }
    }

    fn remote_gesture(&mut self, action: HidAction, ctx: &egui::Context) {
        if self.busy || !self.remote_control_allowed() {
            return;
        }
        self.remote_window.command_started = Some(Instant::now());
        self.dispatch(Action::Gesture(self.selected.clone(), action), ctx);
    }

    pub(super) fn start_live(&mut self, ctx: &egui::Context) {
        if self.busy || !self.api_confirmed || self.selected.is_empty() {
            return;
        }
        let client = match DeviceClient::new(&self.origin, &self.token) {
            Ok(client) => client,
            Err(error) => {
                self.error = Some(error.to_string());
                return;
            }
        };
        self.control_enabled = false;
        self.snapshot_at = None;
        self.remote_drag = None;
        self.texture = None;
        self.remote_window.frame = None;
        self.remote_window.geometry = None;
        self.remote_window.source_size = [0, 0];
        self.remote_window.paused = false;
        self.error = None;
        let id = self.selected.clone();
        let origin = self.origin.clone();
        let context = ctx.clone();
        let (tx, rx) = tokio::sync::watch::channel(State::Connecting);
        let target = id.clone();
        let worker = self.runtime.spawn(async move {
            let result = decoder::run(client, target, |image, source_size, sequence| {
                if tx
                    .send(State::Frame {
                        image: Arc::new(image),
                        sequence,
                        received: Instant::now(),
                        source_size,
                    })
                    .is_err()
                {
                    return false;
                }
                context.request_repaint_of(remote_viewport_id());
                context.request_repaint();
                true
            })
            .await;
            if let Err(error) = result {
                let _ = tx.send(State::Ended(error));
                context.request_repaint();
                context.request_repaint_of(remote_viewport_id());
            }
        });
        self.live_feed = Some(Feed {
            id,
            origin,
            state: rx,
            worker,
            sequence: 0,
            samples: VecDeque::new(),
            uploaded_samples: VecDeque::new(),
            upload_count: 0,
            started: Instant::now(),
            last_metrics: Instant::now(),
            present_reporter: present_reporter(&self.runtime),
        });
        self.log(
            "Tela remota solicitada. Habilite o controle quando a imagem estiver atualizada."
                .into(),
        );
    }

    pub(super) fn stop_live(&mut self) {
        self.live_feed = None;
        self.remote_drag = None;
        self.control_enabled = false;
        self.snapshot_at = None;
    }

    pub(super) fn poll_live(&mut self, ctx: &egui::Context) {
        let Some(feed) = &mut self.live_feed else {
            return;
        };
        if !self.remote_window.open
            || self.selected != feed.id
            || self.origin != feed.origin
            || !self.api_confirmed
        {
            self.stop_live();
            return;
        }
        let state = feed.state.borrow_and_update().clone();
        match state {
            State::Frame {
                image,
                sequence,
                received,
                source_size,
            } if sequence != feed.sequence => {
                feed.sequence = sequence;
                record_sample(&mut feed.samples, received, sequence);
                // The verified canvas belongs to this session and its dimensions.
                // HEVC ringing around a later bright edge must not revoke the
                // user's control opt-in or make the image jump between crops.
                let geometry = if self.image_size == image.size
                    && self.remote_window.source_size == source_size
                    && self.remote_window.geometry.is_some()
                {
                    self.remote_window.geometry
                } else {
                    frame_geometry(image.as_ref(), source_size)
                };
                let verified_source = if geometry.is_some() {
                    source_size
                } else {
                    [0, 0]
                };
                if self.image_size != image.size
                    || self.remote_window.source_size != verified_source
                    || self.remote_window.geometry != geometry
                {
                    self.control_enabled = false;
                    self.remote_drag = None;
                }
                self.image_size = image.size;
                self.remote_window.source_size = verified_source;
                self.remote_window.geometry = geometry;
                self.snapshot_at = Some(received);
                self.remote_window.frame = Some(image.clone());
                if let Some(texture) = &mut self.texture {
                    texture.set(image.clone(), egui::TextureOptions::LINEAR);
                } else {
                    self.texture = Some(ctx.load_texture(
                        "live-phone",
                        image.clone(),
                        egui::TextureOptions::LINEAR,
                    ));
                }
                feed.upload_count += 1;
                record_sample(
                    &mut feed.uploaded_samples,
                    Instant::now(),
                    feed.upload_count,
                );
                if feed.last_metrics.elapsed() >= Duration::from_secs(2) {
                    feed.report_metrics(false);
                    feed.last_metrics = Instant::now();
                }
            }
            State::Ended(error) => {
                self.error = Some(error);
                self.stop_live();
            }
            _ => {}
        }
        if !self.remote_frame_fresh() {
            self.control_enabled = false;
            self.remote_drag = None;
        }
        // Repaints are frame driven. This slow tick only expires stale controls.
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    fn remote_frame_fresh(&self) -> bool {
        self.snapshot_at
            .is_some_and(|at| at.elapsed() < Duration::from_secs(2))
    }

    fn remote_control_allowed(&self) -> bool {
        self.api_confirmed
            && self.live_feed.is_some()
            && self.control_enabled
            && self.remote_frame_fresh()
            && self.remote_window.source_size[0] > 0
            && !self.busy
    }

    pub(super) fn render_remote(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add_space(12.0);
        ui.heading("Seu iPhone, em uma janela própria");
        ui.label("Abra a tela ao vivo e mantenha o espaço de trabalho livre para elementos e diagnósticos.");
        ui.add_space(20.0);
        egui::Frame::new()
            .fill(theme::SURFACE)
            .corner_radius(12)
            .inner_margin(24)
            .show(ui, |ui| {
                ui.strong("Tela remota");
                ui.add_space(8.0);
                ui.label("Clique e arraste na imagem para interagir. Os controles do aparelho ficam ao lado da tela.");
                ui.add_space(16.0);
                if ui.add_enabled(
                    self.api_confirmed && !self.selected.is_empty() && !self.busy,
                    egui::Button::new(if self.remote_window.open { "Mostrar janela da tela" } else { "Abrir tela ao vivo" })
                        .fill(theme::SELECTED),
                ).clicked() {
                    self.open_remote_window(ctx);
                }
                if !self.api_confirmed || self.selected.is_empty() {
                    ui.small("Conecte um aparelho para abrir a transmissão.");
                } else if self.remote_window.open {
                    ui.small("A janela continua aberta enquanto você navega pelas outras seções.");
                }
            });
        ui.add_space(18.0);
        ui.weak("A transmissão começa somente quando você abre a tela. Fechar a janela encerra a captura.");
    }

    pub(super) fn render_remote_window(&mut self, ctx: &egui::Context) {
        if !self.remote_window.open {
            return;
        }
        if !self.api_confirmed
            || self.selected.is_empty()
            || self
                .remote_window
                .target
                .as_ref()
                .is_none_or(|(id, origin)| id != &self.selected || origin != &self.origin)
        {
            self.remote_window.open = false;
            self.stop_live();
            return;
        }
        let device_name = self
            .report
            .as_ref()
            .and_then(|report| {
                report
                    .devices
                    .iter()
                    .find(|device| device.device_id == self.selected)
            })
            .and_then(|device| device.name.as_deref())
            .unwrap_or("iPhone");
        let title = format!("{device_name} · TetherDeck");
        ctx.show_viewport_immediate(
            remote_viewport_id(),
            egui::ViewportBuilder::default()
                .with_title(title)
                .with_inner_size([510.0, 830.0])
                .with_min_inner_size([340.0, 460.0])
                .with_resizable(true),
            |ctx, class| {
                if ctx.input(|input| input.viewport().close_requested()) {
                    self.remote_window.open = false;
                }
                if class == egui::ViewportClass::Embedded {
                    let mut open = self.remote_window.open;
                    egui::Window::new("Tela remota · TetherDeck")
                        .id(egui::Id::new("remote-embedded"))
                        .open(&mut open)
                        .resizable(true)
                        .default_size([480.0, 740.0])
                        .show(ctx, |ui| self.remote_surface(ui, ctx));
                    self.remote_window.open = open;
                } else {
                    egui::TopBottomPanel::bottom("remote-status")
                        .exact_height(32.0)
                        .frame(
                            egui::Frame::new()
                                .fill(theme::CHROME)
                                .inner_margin(egui::Margin::symmetric(12, 4)),
                        )
                        .show(ctx, |ui| self.remote_status(ui));
                    egui::CentralPanel::default()
                        .frame(egui::Frame::new().fill(theme::CHROME).inner_margin(12))
                        .show(ctx, |ui| self.remote_surface(ui, ctx));
                }
            },
        );
        if !self.remote_window.open {
            self.stop_live();
            self.remote_window.paused = false;
            self.remote_window.fullscreen = false;
        }
    }

    fn remote_surface(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let fresh = self.remote_frame_fresh();
        egui::Frame::new()
            .fill(theme::SURFACE)
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .corner_radius(8)
            .inner_margin(8)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), 32.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let running = self.live_feed.is_some();
                        if remote_button(
                            ui,
                            if running {
                                ToolIcon::Pause
                            } else {
                                ToolIcon::Play
                            },
                            None,
                            if running {
                                "Pausar transmissão"
                            } else {
                                "Retomar transmissão"
                            },
                            !self.busy,
                            false,
                            egui::vec2(32.0, 32.0),
                        )
                        .clicked()
                        {
                            if running {
                                self.stop_live();
                                self.remote_window.paused = true;
                            } else {
                                self.start_live(ctx);
                            }
                        }
                        let allow_toggle =
                            running && fresh && self.remote_window.source_size[0] > 0 && !self.busy;
                        if remote_button(
                            ui,
                            ToolIcon::MousePointer,
                            Some("Interagir"),
                            if self.control_enabled {
                                "Desativar interação com o iPhone"
                            } else {
                                "Habilitar clique, arraste e botões do iPhone"
                            },
                            allow_toggle,
                            self.control_enabled,
                            egui::vec2(108.0, 32.0),
                        )
                        .clicked()
                        {
                            self.control_enabled = !self.control_enabled;
                            self.remote_drag = None;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if remote_button(
                                ui,
                                if self.remote_window.fullscreen {
                                    ToolIcon::Minimize
                                } else {
                                    ToolIcon::Maximize
                                },
                                None,
                                if self.remote_window.fullscreen {
                                    "Sair da tela cheia · Esc"
                                } else {
                                    "Ampliar em tela cheia"
                                },
                                true,
                                self.remote_window.fullscreen,
                                egui::vec2(32.0, 32.0),
                            )
                            .clicked()
                            {
                                self.remote_window.fullscreen = !self.remote_window.fullscreen;
                                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(
                                    self.remote_window.fullscreen,
                                ));
                            }
                        });
                    },
                );
            });
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) && self.remote_window.fullscreen
        {
            self.remote_window.fullscreen = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        if self.live_feed.is_some() && fresh && self.remote_window.source_size[0] == 0 {
            ui.small(RichText::new("Controle suspenso: não foi possível confirmar o enquadramento da tela. Confira a orientação do iPhone e retome a transmissão.").color(theme::WARNING));
        }
        if self.live_feed.is_none() && !self.remote_window.paused {
            if let Some(error) = &self.error {
                ui.collapsing("Detalhes da conexão", |ui| {
                    ui.colored_label(theme::ERROR, error);
                });
            }
        }
        ui.add_space(12.0);
        let available = ui.available_size();
        let toolbar_width = 76.0;
        let screen_size = egui::vec2(
            (available.x - toolbar_width - 12.0).max(1.0),
            available.y.max(1.0),
        );
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.allocate_ui_with_layout(
                screen_size,
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    ui.set_min_size(screen_size);
                    let surface = egui::Rect::from_min_size(ui.max_rect().min, screen_size);
                    ui.painter().rect_filled(surface, 8, theme::BACKGROUND);
                    ui.painter().rect_stroke(
                        surface,
                        8,
                        egui::Stroke::new(1.0, theme::BORDER),
                        egui::StrokeKind::Inside,
                    );
                    self.remote_phone_surface(ui, ctx, screen_size);
                },
            );
            ui.allocate_ui_with_layout(
                egui::vec2(toolbar_width, available.y),
                egui::Layout::top_down(egui::Align::Center),
                |ui| {
                    egui::Frame::new()
                        .fill(theme::SURFACE)
                        .stroke(egui::Stroke::new(1.0, theme::BORDER))
                        .corner_radius(8)
                        .inner_margin(8)
                        .show(ui, |ui| {
                            // Frame adds its 1 px stroke to each 8 px inset.
                            ui.set_width(toolbar_width - 18.0);
                            egui::ScrollArea::vertical()
                                .id_salt("remote-controls")
                                .max_height((available.y - 18.0).max(1.0))
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    self.remote_toolbar(ui, ctx);
                                });
                        });
                },
            );
        });
    }

    fn remote_status(&self, ui: &mut egui::Ui) {
        let (status, color, mut details) = if let Some(feed) = &self.live_feed {
            if self.remote_frame_fresh() {
                (
                    format!("●  Ao vivo · {:.1} FPS", sample_fps(&feed.uploaded_samples)),
                    theme::SUCCESS,
                    format!(
                        "Quadros enviados à textura da interface nos últimos 2 segundos. Decodificação: {:.1} FPS. A apresentação final pela GPU não é medida.",
                        sample_fps(&feed.samples)
                    ),
                )
            } else {
                (
                    "Aguardando imagem…".into(),
                    theme::MUTED,
                    "Aguardando um quadro atual do aparelho.".into(),
                )
            }
        } else if self.remote_window.paused {
            (
                "Pausado · controle desativado".into(),
                theme::MUTED,
                "Retome a transmissão para receber imagens e habilitar o controle.".into(),
            )
        } else {
            (
                "Transmissão encerrada".into(),
                theme::MUTED,
                "Use Retomar para abrir uma nova sessão de vídeo.".into(),
            )
        };
        if let Some(ms) = self.remote_window.command_ms {
            details.push_str(&format!(
                "\nÚltimo comando: {ms} ms até a resposta. Não mede a latência do vídeo."
            ));
        }
        if let Some(at) = self.snapshot_at {
            details.push_str(&format!(
                "\nQuadro decodificado há {} ms. Não mede o atraso desde a tela do iPhone.",
                at.elapsed().as_millis()
            ));
        }
        let show_details = ui.available_width() >= 560.0;
        ui.horizontal(|ui| {
            let main_width = (ui.available_width() - if show_details { 290.0 } else { 0.0 }).max(1.0);
            ui.add_sized(
                [main_width, 18.0],
                egui::Label::new(RichText::new(status).color(color)).truncate(),
            ).on_hover_text(details);
            if show_details {
                if let Some(ms) = self.remote_window.command_ms {
                    ui.small(format!("· comando {ms} ms"))
                        .on_hover_text("Tempo até a resposta do comando. Não mede a latência do vídeo.");
                }
                if let Some(at) = self.snapshot_at {
                    ui.small(format!("· quadro há {} ms", at.elapsed().as_millis()))
                        .on_hover_text("Tempo desde a saída do decoder. Não mede o atraso desde a tela do iPhone.");
                }
            }
        });
    }

    fn remote_toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let controls = [
            (ToolIcon::Home, "Início", "Tela inicial", DeviceButton::Home),
            (
                ToolIcon::AppWindow,
                "Apps",
                "Alternador de aplicativos",
                DeviceButton::AppSwitcher,
            ),
            (
                ToolIcon::VolumeUp,
                "Vol +",
                "Aumentar volume",
                DeviceButton::VolumeUp,
            ),
            (
                ToolIcon::VolumeDown,
                "Vol −",
                "Diminuir volume",
                DeviceButton::VolumeDown,
            ),
            (
                ToolIcon::Lock,
                "Bloquear",
                "Bloquear a tela do iPhone",
                DeviceButton::Lock,
            ),
        ];
        for (index, (icon, label, tooltip, button)) in controls.into_iter().enumerate() {
            if index == 2 || index == 4 {
                remote_group_separator(ui);
            }
            if tool_button(ui, icon, label, tooltip, self.remote_control_allowed()) {
                self.remote_gesture(HidAction::DeviceButton { button }, ctx);
                if matches!(button, DeviceButton::Lock) {
                    self.control_enabled = false;
                }
            }
            ui.add_space(4.0);
        }
        remote_group_separator(ui);
        if tool_button(
            ui,
            ToolIcon::Camera,
            "Captura",
            "Copiar o quadro exibido para a área de transferência",
            self.remote_window.frame.is_some(),
        ) {
            if let Some(image) = &self.remote_window.frame {
                let copied = if let Some(geometry) = self.remote_window.geometry {
                    let [width, height] = geometry.content_size;
                    if geometry.content_size != image.size {
                        // An explicit one-off clipboard copy crops CPU pixels;
                        // continuous display only changes texture UV coordinates.
                        egui::ColorImage::new(
                            geometry.content_size,
                            image
                                .pixels
                                .chunks_exact(image.size[0])
                                .take(height)
                                .flat_map(|row| row[..width].iter().copied())
                                .collect(),
                        )
                    } else {
                        image.as_ref().clone()
                    }
                } else {
                    image.as_ref().clone()
                };
                ctx.copy_image(copied);
                self.log("Quadro exibido copiado para a área de transferência.".into());
            }
        }
        if self.busy {
            ui.spinner();
        }
    }

    fn remote_phone_surface(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, bounds: egui::Vec2) {
        let Some(texture) = self.texture.clone() else {
            ui.add_space((bounds.y * 0.32).max(0.0));
            if self.live_feed.is_some() {
                ui.spinner();
                ui.strong("Abrindo transmissão");
                ui.weak("Aguardando o primeiro quadro do aparelho.");
            } else {
                ui.strong("Tela indisponível");
                if let Some(error) = &self.error {
                    ui.label(error);
                }
                ui.weak("Retome a transmissão para tentar uma nova sessão.");
            }
            return;
        };
        let content_size = self
            .remote_window
            .geometry
            .map(|geometry| geometry.content_size)
            .unwrap_or(self.image_size);
        let native = egui::vec2(content_size[0].max(1) as f32, content_size[1].max(1) as f32);
        let uv = self
            .remote_window
            .geometry
            .map(|geometry| geometry.uv)
            .unwrap_or(egui::Rect::from_min_max(
                egui::Pos2::ZERO,
                egui::pos2(1.0, 1.0),
            ));
        let scale = (bounds.x / native.x).min(bounds.y / native.y);
        let size = native * scale;
        ui.add_space(((bounds.y - size.y) / 2.0).max(0.0));
        let response = ui.add(
            egui::Image::new(&texture)
                .uv(uv)
                .fit_to_exact_size(size)
                .sense(egui::Sense::click_and_drag()),
        ).on_hover_text("Arrastes são enviados ao soltar o mouse.");
        let allowed = self.remote_control_allowed();
        if allowed {
            response
                .clone()
                .on_hover_cursor(egui::CursorIcon::PointingHand);
        }
        let point = |position: egui::Pos2| {
            let uv = (position - response.rect.min) / response.rect.size();
            (
                (uv.x * self.remote_window.source_size[0] as f32)
                    .floor()
                    .clamp(
                        0.0,
                        self.remote_window.source_size[0].saturating_sub(1) as f32,
                    ) as u16,
                (uv.y * self.remote_window.source_size[1] as f32)
                    .floor()
                    .clamp(
                        0.0,
                        self.remote_window.source_size[1].saturating_sub(1) as f32,
                    ) as u16,
            )
        };
        if response.drag_started() && allowed {
            if let Some(position) = ctx.input(|input| input.pointer.press_origin()) {
                if response.rect.contains(position) {
                    self.remote_drag = Some((point(position), Instant::now(), self.image_size));
                }
            }
        }
        if response.drag_stopped() {
            if let Some((start, at, size)) = self.remote_drag.take() {
                if allowed && size == self.image_size && at.elapsed() < Duration::from_secs(5) {
                    if let Some(position) = response.interact_pointer_pos() {
                        let end = point(position);
                        self.remote_gesture(
                            HidAction::Drag {
                                start_x: start.0,
                                start_y: start.1,
                                end_x: end.0,
                                end_y: end.1,
                                duration_ms: at.elapsed().as_millis().clamp(100, 250) as u32,
                            },
                            ctx,
                        );
                    }
                }
            }
        } else if response.clicked() && allowed {
            if let Some(position) = response.interact_pointer_pos() {
                let (x, y) = point(position);
                self.remote_gesture(HidAction::Tap { x, y }, ctx);
            }
        }
        if !allowed {
            self.remote_drag = None;
        }
    }
}

fn remote_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("tetherdeck-remote-screen")
}

fn remote_group_separator(ui: &mut egui::Ui) {
    ui.add_space(4.0);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().line_segment(
        [
            egui::pos2(rect.center().x - 16.0, rect.center().y),
            egui::pos2(rect.center().x + 16.0, rect.center().y),
        ],
        egui::Stroke::new(1.0, theme::BORDER),
    );
    ui.add_space(8.0);
}

fn remote_button(
    ui: &mut egui::Ui,
    icon: ToolIcon,
    label: Option<&str>,
    tooltip: &str,
    enabled: bool,
    selected: bool,
    size: egui::Vec2,
) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| {
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        let hover = ui.ctx().animate_bool_with_time(
            response.id.with("remote-hover"),
            response.hovered() && enabled,
            0.12,
        );
        let base = if selected {
            theme::SELECTED
        } else {
            theme::SURFACE
        };
        let fill = base.lerp_to_gamma(theme::SELECTED, hover);
        ui.painter().rect_filled(rect, 6, fill);
        let border = if selected {
            theme::ACCENT
        } else {
            theme::BORDER
        };
        ui.painter().rect_stroke(
            rect,
            6,
            egui::Stroke::new(1.0, border),
            egui::StrokeKind::Inside,
        );
        let tint = if !enabled {
            theme::MUTED.gamma_multiply(0.5)
        } else if selected || response.hovered() {
            theme::TEXT
        } else {
            theme::MUTED
        };
        let icon_size = if size.y >= 44.0 { 20.0 } else { 16.0 };
        let center = if label.is_some() {
            rect.left_center() + egui::vec2(18.0, 0.0)
        } else {
            rect.center()
        };
        let texture = icons::texture(ui.ctx(), icon);
        ui.painter().image(
            texture.id(),
            egui::Rect::from_center_size(center, egui::vec2(icon_size, icon_size)),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            tint,
        );
        if let Some(label) = label {
            ui.painter().text(
                rect.left_center() + egui::vec2(34.0, 0.0),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::proportional(12.0),
                tint,
            );
        }
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, tooltip)
        });
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tooltip)
    })
    .inner
}

fn tool_button(
    ui: &mut egui::Ui,
    icon: ToolIcon,
    label: &str,
    tooltip: &str,
    enabled: bool,
) -> bool {
    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        let clicked = remote_button(
            ui,
            icon,
            None,
            tooltip,
            enabled,
            false,
            egui::vec2(44.0, 44.0),
        )
        .clicked();
        ui.add_sized(
            [44.0, 12.0],
            egui::Label::new(RichText::new(label).size(10.0).color(if enabled {
                theme::MUTED
            } else {
                theme::MUTED.gamma_multiply(0.5)
            }))
            .halign(egui::Align::Center)
            .truncate(),
        );
        clicked
    })
    .inner
}
