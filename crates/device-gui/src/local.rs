//! Local diagnostics and explicit recovery remain usable without an HTTP server or token.
use super::*;

pub(super) enum LocalRecovery {
    Service,
    Usb {
        device_id: String,
        instance_id: String,
    },
}

pub(super) async fn local_command(
    args: &[&str],
    seconds: u64,
    recovery: bool,
) -> Result<Vec<u8>, String> {
    let mut daemon = std::env::current_exe()
        .map_err(|_| "Não foi possível localizar esta interface.".to_owned())?;
    daemon.set_file_name(if cfg!(windows) {
        "rust-ios-device-daemon.exe"
    } else {
        "rust-ios-device-daemon"
    });
    let mut command = tokio::process::Command::new(daemon);
    command.args(args).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW; no shell or console is opened.
    let output = tokio::time::timeout(Duration::from_secs(seconds), command.output())
        .await
        .map_err(|_| {
            if recovery {"A recuperação local excedeu o prazo; o resultado pode ser incerto. Faça um novo diagnóstico antes de repetir.".to_owned()}
            else {"O diagnóstico local excedeu o prazo. A interface continua disponível.".to_owned()}
        })?
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => {
                "Mantenha o executável rust-ios-device-daemon na mesma pasta da interface."
                    .to_owned()
            }
            std::io::ErrorKind::PermissionDenied => {
                "O sistema negou a execução do diagnóstico local.".to_owned()
            }
            _ => "Não foi possível iniciar o diagnóstico local.".to_owned(),
        })?;
    if output.stdout.len() > 1024 * 1024 {
        return Err(
            "A resposta local excedeu o limite. Confira o diagnóstico antes de repetir.".into(),
        );
    }
    // Recovery may exit nonzero while returning a useful typed outcome such as
    // permission_required or reboot_required. Preserve that evidence for the GUI.
    Ok(output.stdout)
}
async fn local_report(
    mode: u8,
    id: String,
    origin: String,
    host: String,
) -> Result<DiagnosticReport, String> {
    let mut args = vec![
        "--doctor",
        "--transport",
        if mode == 1 { "wifi" } else { "usb" },
    ];
    if mode == 1 && !id.trim().is_empty() {
        args.extend(["--wifi-direct", "--device", &id]);
    }
    if mode == 1 && !host.trim().is_empty() {
        args.extend(["--wifi-host", &host]);
    }
    if mode == 2 {
        args.extend(["--device", &id, "--tunneld-url", &origin]);
    }
    serde_json::from_slice(&local_command(&args, 25, false).await?)
        .map_err(|_| "Resposta local incompatível. Use os executáveis da mesma versão.".into())
}
impl Studio {
    pub(super) fn local_recovery_available(&self) -> bool {
        !self.api_confirmed
            && self.report_label == "Computador local"
            && self.report.is_some()
            && self.local_connection.is_none()
    }
    pub(super) fn local_recovery_remaining(&self) -> u64 {
        self.last_local_recovery
            .map(|at| 60_u64.saturating_sub(at.elapsed().as_secs()))
            .unwrap_or(0)
    }
    pub(super) fn recover_local(&mut self, recovery: LocalRecovery, ctx: &egui::Context) {
        if self.busy || !self.local_recovery_available() || self.local_recovery_remaining() > 0 {
            return;
        }
        self.last_local_recovery = Some(Instant::now());
        self.busy = true;
        self.error = None;
        self.api_confirmed = false;
        self.runner_configured = false;
        self.usb_recovery_enabled = false;
        self.selected.clear();
        self.selected_element.clear();
        self.elements = None;
        self.texture = None;
        self.snapshot_at = None;
        self.control_enabled = false;
        self.page = 2;
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        let context = ctx.clone();
        self.runtime.spawn(async move {
            let event: Result<Event, String> = async {
                match recovery {
                    LocalRecovery::Service => {
                        let bytes = local_command(&["--recover-service"], 50, true).await?;
                        serde_json::from_slice(&bytes).map(Event::Recovered)
                            .map_err(|_| "Resultado local incompatível. Consulte o diagnóstico antes de repetir.".into())
                    }
                    LocalRecovery::Usb { device_id, instance_id } => {
                        let bytes = local_command(&["--device", &device_id, "--recover-usb-interface", &instance_id], 45, true).await?;
                        serde_json::from_slice(&bytes).map(Event::UsbRecovered)
                            .map_err(|_| "Resultado USB local incompatível. Consulte o diagnóstico antes de repetir.".into())
                    }
                }
            }.await;
            let _ = sender.send(event.unwrap_or_else(Event::Failed));
            context.request_repaint();
        });
    }
    pub(super) fn diagnose_local(&mut self, ctx: &egui::Context) {
        if self.busy {
            return;
        }
        self.api_confirmed = false;
        self.runner_configured = false;
        self.report_label = "Computador local";
        self.report = None;
        self.selected.clear();
        self.selected_element.clear();
        self.elements = None;
        self.texture = None;
        self.snapshot_at = None;
        self.control_enabled = false;
        self.error = None;
        self.busy = true;
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        let context = ctx.clone();
        let mode = self.connection_mode;
        let id = self.wifi_udid.clone();
        let origin = self.tunneld_origin.clone();
        let host = self.wifi_host.clone();
        self.runtime.spawn(async move {
            let event = match local_report(mode, id, origin, host).await {
                Ok(report) => Event::LocalReport(report),
                Err(error) => Event::Failed(error),
            };
            let _ = sender.send(event);
            context.request_repaint();
        });
    }
}
