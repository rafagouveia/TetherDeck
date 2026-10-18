#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use eframe::egui::{self, Color32, RichText};
use rust_ios_device_api::{CheckStatus, DiagnosticReport, HidAction, RecoveryReport, UiSnapshot};
use rust_ios_device_client::DeviceClient;
use std::{
    collections::VecDeque,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

const ACCENT: Color32 = theme::ACCENT;
mod components;
mod connection;
mod decoder;
mod elements;
mod icons;
mod live;
mod local;
mod theme;
mod workbench;
mod workspace;
enum Event {
    Connected(connection::LocalConnection, DiagnosticReport),
    LocalReport(DiagnosticReport),
    Elements(UiSnapshot, bool),
    Report(DiagnosticReport, bool, bool),
    UsbRecovered(rust_ios_device_api::UsbRecoveryReport),
    Recovered(RecoveryReport),
    Message(String),
    Failed(String),
}
enum Action {
    Elements(String),
    Activate(String, String, String),
    Diagnose,
    Gesture(String, HidAction),
    Restart,
    RestartElements,
    RestartUsb(String, String),
}

struct Studio {
    remote_window: live::RemoteWindow,
    live_feed: Option<live::Feed>,
    remote_drag: Option<((u16, u16), Instant, [usize; 2])>,
    brand_texture: egui::TextureHandle,
    connection_mode: u8,
    wifi_udid: String,
    wifi_host: String,
    tunneld_origin: String,
    activity_open: bool,
    last_local_recovery: Option<Instant>,
    local_allow_usb_recovery: bool,
    usb_recovery_enabled: bool,
    local_connection: Option<connection::LocalConnection>,
    runner_bundle: String,
    api_confirmed: bool,
    report_label: &'static str,
    #[cfg(feature = "capture-ui")]
    capture: Option<(std::path::PathBuf, Instant, bool)>,
    runner_configured: bool,
    page: u8,
    elements: Option<Arc<UiSnapshot>>,
    element_filter: String,
    selected_element: String,
    named_only: bool,
    visible_only: bool,
    origin: String,
    token: String,
    report: Option<DiagnosticReport>,
    selected: String,
    busy: bool,
    error: Option<String>,
    events: VecDeque<String>,
    receiver: Option<mpsc::Receiver<Event>>,
    texture: Option<egui::TextureHandle>,
    image_size: [usize; 2],
    control_enabled: bool,
    snapshot_at: Option<Instant>,
    runtime: tokio::runtime::Runtime,
}

impl Studio {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let mut studio = Self {
            remote_window: live::RemoteWindow::default(),
            live_feed: None,
            remote_drag: None,
            brand_texture: {
                let bytes = image::load_from_memory(include_bytes!(
                    "../../../assets/brand/tetherdeck-icon-256.png"
                ))
                .expect("bundled brand image")
                .to_rgba8();
                cc.egui_ctx.load_texture(
                    "tetherdeck-brand",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [bytes.width() as usize, bytes.height() as usize],
                        bytes.as_raw(),
                    ),
                    egui::TextureOptions::LINEAR,
                )
            },
            connection_mode: 0,
            wifi_udid: connection::known_wifi_device(),
            wifi_host: String::new(),
            tunneld_origin: "http://127.0.0.1:49151".into(),
            activity_open: false,
            last_local_recovery: None,
            local_allow_usb_recovery: false,
            usb_recovery_enabled: false,
            local_connection: None,
            runner_bundle: "io.rustiosdevice.wda.runner.xctrunner".into(),
            api_confirmed: false,
            report_label: "Host do daemon",
            #[cfg(feature = "capture-ui")]
            capture: std::env::var_os("RUST_IOS_DEVICE_CAPTURE")
                .map(|path| (path.into(), Instant::now(), false)),
            runner_configured: false,
            page: 3,
            elements: None,
            element_filter: String::new(),
            selected_element: String::new(),
            named_only: true,
            visible_only: true,
            origin: std::env::var("RUST_IOS_DEVICE_URL").unwrap_or("http://127.0.0.1:18087".into()),
            token: std::env::var("RUST_IOS_DEVICE_TOKEN").unwrap_or_default(),
            report: None,
            selected: String::new(),
            busy: false,
            error: None,
            events: VecDeque::new(),
            receiver: None,
            texture: None,
            image_size: [0, 0],
            control_enabled: false,
            snapshot_at: None,
            runtime: tokio::runtime::Runtime::new().expect("background runtime"),
        };
        if let Ok(path) = std::env::var("RUST_IOS_DEVICE_REPORT") {
            match std::fs::read(path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<DiagnosticReport>(&bytes).ok())
            {
                Some(report) => {
                    studio.page = 2;
                    studio.report_label = "Relatório salvo";
                    studio.report = Some(report);
                    studio.log("Relatório salvo carregado. Diagnosticar conexão consulta o estado atual do host.".into());
                }
                None => {
                    studio.error =
                        Some("Não foi possível ler o relatório de diagnóstico informado.".into())
                }
            }
        } else if !studio.token.trim().is_empty() {
            studio.dispatch(Action::Diagnose, &cc.egui_ctx);
        } else {
            studio.diagnose_local(&cc.egui_ctx);
        }
        studio
    }

    fn log(&mut self, message: String) {
        self.events.push_front(message);
        self.events.truncate(8);
    }

    #[cfg(feature = "capture-ui")]
    fn export_frame(&mut self, ctx: &egui::Context) {
        let Some((path, started, requested)) = &mut self.capture else {
            return;
        };
        let screenshot = ctx.input(|input| {
            input.raw.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(screenshot) = screenshot {
            let bytes: Vec<u8> = screenshot
                .pixels
                .iter()
                .flat_map(|pixel| pixel.to_array())
                .collect();
            match image::save_buffer(
                path,
                &bytes,
                screenshot.width() as u32,
                screenshot.height() as u32,
                image::ColorType::Rgba8,
            ) {
                Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Err(error) => self.error = Some(format!("Falha ao exportar interface: {error}")),
            };
            self.capture = None;
            return;
        }
        if !*requested && !self.busy && started.elapsed() >= Duration::from_secs(1) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            *requested = true;
        }
        ctx.request_repaint_after(Duration::from_millis(50));
    }

    fn dispatch(&mut self, action: Action, ctx: &egui::Context) {
        if self.busy {
            return;
        }
        let client = match DeviceClient::new(&self.origin, &self.token) {
            Ok(c) => c,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        self.busy = true;
        self.error = None;
        if matches!(
            &action,
            Action::Gesture(..)
                | Action::Activate(..)
                | Action::Restart
                | Action::RestartElements
                | Action::RestartUsb(..)
                | Action::Elements(..)
        ) {
            if self.live_feed.is_none() {
                self.snapshot_at = None;
                self.control_enabled = false;
            }
            self.elements = None;
            self.selected_element.clear();
        }
        let context = ctx.clone();
        self.runtime.spawn(async move {
            let result: Result<Event, rust_ios_device_client::ClientError> = async {
                client.health().await?;
                match action {
                    Action::RestartUsb(device_id, instance_id) => Ok(Event::UsbRecovered(
                        client
                            .recover_usb_interface(&rust_ios_device_api::UsbRecoveryRequest {
                                device_id,
                                instance_id,
                            })
                            .await?,
                    )),
                    Action::Elements(id) => {
                        Ok(Event::Elements(client.ui_snapshot(&id).await?, false))
                    }
                    Action::Activate(id, snapshot, element) => {
                        client
                            .activate_element(
                                &id,
                                &rust_ios_device_api::UiActivateRequest {
                                    snapshot_id: snapshot,
                                    element_id: element,
                                },
                            )
                            .await?;
                        Ok(Event::Elements(client.ui_snapshot(&id).await?, true))
                    }
                    Action::Diagnose => {
                        let capabilities = client.capabilities().await?;
                        let configured = capabilities
                            .pointer("/features/ui_elements/runner_configured")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false);
                        let usb = capabilities
                            .get("usb_interface_restart")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false);
                        Ok(Event::Report(client.diagnostics().await?, configured, usb))
                    }
                    Action::RestartElements => {
                        client.recover_elements().await?;
                        Ok(Event::Message(
                            "Runner reiniciado e WDA disponível. Consulte os elementos novamente."
                                .into(),
                        ))
                    }
                    Action::Gesture(id, action) => {
                        client.action(&id, &action).await?;
                        Ok(Event::Message(
                            "Gesto enviado. Confira o resultado na tela.".into(),
                        ))
                    }
                    Action::Restart => {
                        let result = client.recover_service().await?;
                        Ok(Event::Recovered(result))
                    }
                }
            }
            .await;
            let _ = sender.send(result.unwrap_or_else(|error| Event::Failed(error.to_string())));
            context.request_repaint();
        });
    }

    fn poll(&mut self) {
        let events: Vec<Event> = self
            .receiver
            .as_ref()
            .map(|r| r.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            match event {
                Event::Connected(connection, report) => {
                    self.usb_recovery_enabled = self.local_allow_usb_recovery && cfg!(windows);
                    self.origin = connection.origin.clone();
                    self.token = connection.token.clone();
                    let runner_configured = connection.runner_configured;
                    let initial_page = 3;
                    self.local_connection = Some(connection);
                    self.api_confirmed = true;
                    self.runner_configured = runner_configured;
                    self.report_label = "Host do daemon local";
                    self.selected = report
                        .devices
                        .first()
                        .map(|d| d.device_id.clone())
                        .unwrap_or_default();
                    self.report = Some(report);
                    self.page = initial_page;
                    self.busy = false;
                    self.log("Conexão aberta. Use Abrir tela para iniciar a janela remota.".into());
                }
                Event::LocalReport(report) => {
                    self.log(format!(
                        "Diagnóstico deste computador: {} em {} ms.",
                        report.health, report.elapsed_ms
                    ));
                    self.report = Some(report);
                    self.busy = false;
                }
                Event::Elements(snapshot, after_action) => {
                    self.log(if after_action {
                        "Ativação enviada; consulta atualizada para conferir o resultado.".into()
                    } else {
                        format!(
                            "{} elementos consultados via {}.",
                            snapshot.elements.len(),
                            snapshot.provider
                        )
                    });
                    self.elements = Some(Arc::new(snapshot));
                    self.busy = false;
                }
                Event::Report(report, configured, usb) => {
                    self.usb_recovery_enabled = usb;
                    self.api_confirmed = true;
                    self.report_label = "Host do daemon";
                    self.runner_configured = configured;
                    if !report.devices.iter().any(|d| d.device_id == self.selected) {
                        self.selected = report
                            .devices
                            .first()
                            .map(|d| d.device_id.clone())
                            .unwrap_or_default();
                        self.texture = None;
                        self.elements = None;
                    }
                    self.log(format!(
                        "Diagnóstico concluído em {} ms: {}.",
                        report.elapsed_ms, report.health
                    ));
                    self.report = Some(report);
                    self.busy = false;
                }
                Event::UsbRecovered(recovery) => {
                    if recovery.outcome != "completed" {
                        self.error = Some(if recovery.outcome == "reboot_required" {
                            "O Windows exige reinicialização para concluir a recuperação USB. Salve seu trabalho; o programa não reinicia o computador.".into()
                        } else {
                            format!(
                                "{}: recuperação USB {}. Confira privilégios e diagnóstico antes de repetir.",
                                recovery.code, recovery.outcome
                            )
                        });
                    }
                    self.log(format!("Recuperação USB: {} · {}. Consulte a conexão e reinicie o WDA quando o aparelho reaparecer.", recovery.outcome, recovery.code));
                    self.report = Some(recovery.diagnostics);
                    self.busy = false;
                    self.page = 2;
                }
                Event::Message(message) => {
                    self.remote_command_finished();
                    self.log(message);
                    self.busy = false;
                }
                Event::Recovered(result) => {
                    if result.outcome != "completed" {
                        self.error = Some(if result.code == "permission_required" {
                            "A recuperação foi recusada por falta de privilégios. No Windows, abra o programa como administrador; no Linux, confira as permissões do serviço.".into()
                        } else {
                            format!(
                                "{}: recuperação {}. Confira o diagnóstico antes de repetir.",
                                result.code, result.outcome
                            )
                        });
                    }
                    if !result
                        .diagnostics
                        .devices
                        .iter()
                        .any(|d| d.device_id == self.selected)
                    {
                        self.selected.clear();
                    }
                    self.log(format!(
                        "Recuperação: {} / {}. Estado do aparelho: {}.",
                        result.outcome, result.code, result.diagnostics.health
                    ));
                    self.report = Some(result.diagnostics);
                    self.texture = None;
                    self.snapshot_at = None;
                    self.control_enabled = false;
                    self.busy = false;
                }
                Event::Failed(error) => {
                    self.remote_command_finished();
                    self.error = Some(error);
                    self.control_enabled = false;
                    self.remote_drag = None;
                    self.busy = false;
                }
            }
        }
    }
}

impl eframe::App for Studio {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(feature = "capture-ui")]
        self.export_frame(ctx);
        self.poll();
        self.poll_live(ctx);
        self.render_shell(ctx);
        self.render_remote_window(ctx);
    }
}

fn main() -> eframe::Result {
    let icon = image::load_from_memory(include_bytes!(
        "../../../assets/brand/tetherdeck-icon-256.png"
    ))
    .expect("bundled window icon")
    .to_rgba8();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(egui::IconData {
                width: icon.width(),
                height: icon.height(),
                rgba: icon.into_raw(),
            })
            .with_inner_size([1250.0, 820.0])
            .with_min_inner_size([640.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "TetherDeck · Device workspace",
        options,
        Box::new(|cc| Ok(Box::new(Studio::new(cc)))),
    )
}
