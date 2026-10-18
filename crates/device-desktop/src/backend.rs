use crate::runner_import::{self, PreparedRunner};
use gpui_kit::Context;
use rust_ios_device_api::{
    DeviceSummary, DiagnosticReport, ElementProvider, IdentityStatus, RecoveryReport,
    UiActivateRequest, UiSnapshot, UsbRecoveryReport, UsbRecoveryRequest,
};
use rust_ios_device_client::DeviceClient;
use std::{
    collections::VecDeque,
    future::Future,
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Transport {
    #[default]
    Usb,
    Wifi,
    Rsd,
}
impl Transport {
    pub fn label(self) -> &'static str {
        match self {
            Self::Usb => "USB",
            Self::Wifi => "Wi-Fi direto",
            Self::Rsd => "Túnel RSD",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Config {
    pub element_provider: ElementProvider,
    pub transport: Transport,
    pub udid: String,
    pub host: String,
    pub tunneld_origin: String,
    pub runner_bundle: String,
    /// Exact bundle reviewed for USE_IP HTTP+MJPEG binding. Clear on new IPA or bundle edit.
    pub runner_tunnel_binding_reviewed_bundle: String,
    pub allow_usb_recovery: bool,
    pub allow_service_recovery: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            element_provider: ElementProvider::Auto,
            transport: Transport::Usb,
            udid: known_wifi_device(),
            host: String::new(),
            tunneld_origin: "http://127.0.0.1:49151".into(),
            runner_bundle: String::new(),
            runner_tunnel_binding_reviewed_bundle: String::new(),
            allow_usb_recovery: false,
            allow_service_recovery: false,
        }
    }
}

/// Shared ownership keeps the GUI-owned daemon alive while a remote command finishes.
pub struct Session {
    pub client: DeviceClient,
    pub device_id: String,
    pub name: String,
    /// Hardware identifier returned by the connected device, never inferred from pixels.
    pub product_type: Option<String>,
    pub origin: String,
    pub token: String,
    pub transport: Transport,
    pub runner_configured: bool,
    pub allow_usb_recovery: bool,
    pub allow_service_recovery: bool,
    _child: Option<tokio::process::Child>,
}
impl Session {
    pub fn transport_label(&self) -> &'static str {
        if self._child.is_none() {
            "Servidor existente"
        } else {
            self.transport.label()
        }
    }
}

pub struct Model {
    pub element_provider: ElementProvider,
    pub runtime: Arc<tokio::runtime::Runtime>,
    pub busy: bool,
    /// Quiet discovery has its own failure/backoff state; never overwrites a user error.
    pub discovery_failed: bool,
    pub error: Option<String>,
    pub status: String,
    pub report: Option<DiagnosticReport>,
    pub session: Option<Arc<Session>>,
    pub elements: Option<Arc<UiSnapshot>>,
    pub events: VecDeque<String>,
    pub capabilities: Option<serde_json::Value>,
    pub runner_package: Option<Arc<PreparedRunner>>,
    pub installed_runner_bundle: Option<String>,
    generation: u64,
    recovery_at: Option<Instant>,
    last_config: Config,
    discovery_sequence: u64,
    discovery_config: Option<Config>,
    discovery_job: Option<DiscoveryJob>,
}

/// Dropping a GPUI task alone would detach its Tokio JoinHandle. Abort both.
/// The read-only doctor's Command uses kill_on_drop, so dropping that future
/// also terminates its child process instead of leaving a background daemon.
struct DiscoveryJob {
    worker: tokio::task::AbortHandle,
    _ui: gpui_kit::Task<()>,
}
impl Drop for DiscoveryJob {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

enum Update {
    Report(DiagnosticReport),
    Session(Arc<Session>, DiagnosticReport, serde_json::Value),
    Elements(UiSnapshot),
    Message(String),
    Recovery(DiagnosticReport, String, Option<String>),
    RunnerPrepared(Arc<PreparedRunner>),
    RunnerInstalled(runner_import::InstallResult),
}

impl Model {
    pub fn new(runtime: Arc<tokio::runtime::Runtime>, _cx: &mut Context<Self>) -> Self {
        Self {
            runtime,
            element_provider: ElementProvider::Auto,
            busy: false,
            discovery_failed: false,
            error: None,
            status: "Pronto para conectar".into(),
            report: None,
            session: None,
            elements: None,
            events: VecDeque::new(),
            capabilities: None,
            runner_package: None,
            installed_runner_bundle: None,
            generation: 0,
            recovery_at: None,
            last_config: Config::default(),
            discovery_sequence: 0,
            discovery_config: None,
            discovery_job: None,
        }
    }
    fn perform(
        &mut self,
        label: &str,
        cx: &mut Context<Self>,
        work: impl Future<Output = Result<Update, String>> + Send + 'static,
    ) {
        if self.busy {
            return;
        }
        // Foreground commands always supersede read-only automatic discovery.
        self.cancel_discovery();
        self.busy = true;
        self.error = None;
        self.status = label.into();
        let generation = self.generation;
        let task = self.runtime.spawn(work);
        cx.spawn(async move |this, cx| {
            let result = match task.await {
                Ok(result) => result,
                Err(_) => Err("O trabalhador encerrou antes de concluir. Consulte o diagnóstico antes de repetir uma ação.".into()),
            };
            if let Err(error) = this.update(cx, |model, cx| {
                if model.generation != generation { return; }
                model.busy = false;
                match result {
                    Ok(Update::Report(report)) => {
                        model.status = format!("Diagnóstico: {} · {} ms", report.health, report.elapsed_ms);
                        model.report = Some(report);
                    }
                    Ok(Update::Session(session, report, capabilities)) => {
                        model.status = format!("Conectado · {}", session.transport_label());
                        model.session = Some(session);
                        model.report = Some(report);
                        model.capabilities = Some(capabilities);
                    }
                    Ok(Update::Elements(snapshot)) => {
                        model.status = format!("{} elementos · {}", snapshot.elements.len(), snapshot.provider);
                        model.elements = Some(Arc::new(snapshot));
                    }
                    Ok(Update::Message(message)) => model.status = message,
                    Ok(Update::Recovery(report, status, error)) => {
                        model.report = Some(report);
                        model.status = status;
                        model.error = error;
                    }
                    Ok(Update::RunnerPrepared(package)) => {
                        model.runner_package = Some(package);
                        model.status = "IPA preparado. Revise o perfil e o aparelho antes de instalar.".into();
                    }
                    Ok(Update::RunnerInstalled(result)) => {
                        model.last_config.runner_bundle = result.bundle_id.clone();
                        model.last_config.runner_tunnel_binding_reviewed_bundle.clear();
                        model.installed_runner_bundle = Some(result.bundle_id);
                        model.runner_package = None;
                        model.status = "Runner instalado por USB. Reconecte a sessão para ativar a automação.".into();
                        if result.cleanup_warning {
                            model.error = Some("A instalação foi confirmada, mas o arquivo temporário no iPhone pode ter permanecido.".into());
                        }
                    }
                    Err(error) => {
                        model.status = "Operação não concluída".into();
                        model.error = Some(error);
                    }
                }
                model.events.push_front(model.status.clone());
                model.events.truncate(40);
                cx.notify();
            }) { eprintln!("TetherDeck: resultado descartado após fechar a interface: {error}"); }
        }).detach();
        cx.notify();
    }

    pub fn prepare_runner(&mut self, path: PathBuf, target_udid: String, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.runner_package = None;
        self.installed_runner_bundle = None;
        self.last_config
            .runner_tunnel_binding_reviewed_bundle
            .clear();
        self.perform(
            "Verificando IPA e perfil de desenvolvimento…",
            cx,
            async move {
                tokio::time::timeout(
                    Duration::from_secs(60),
                    runner_import::prepare(path, target_udid),
                )
                .await
                .map_err(|_| "A análise do IPA excedeu o prazo.".to_owned())?
                .map(Update::RunnerPrepared)
            },
        );
    }

    pub fn install_runner(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(package) = self.runner_package.clone() else {
            self.error = Some("Escolha e prepare um IPA assinado primeiro.".into());
            cx.notify();
            return;
        };
        self.cancel_discovery();
        let runtime = self.runtime.clone();
        let (send, receive) = tokio::sync::oneshot::channel();
        // Keep Runtime outside the async scope, as for live input. Closing the
        // interface must not cancel installation halfway through staging cleanup.
        if std::thread::Builder::new()
            .name("tetherdeck-runner-install".into())
            .spawn(move || {
                let result = runtime.block_on(runner_import::install(package));
                let _ = send.send(result);
            })
            .is_err()
        {
            self.error = Some("Não foi possível iniciar o trabalhador de instalação.".into());
            cx.notify();
            return;
        }
        self.perform("Instalando runner no iPhone por USB…", cx, async move {
            receive.await.map_err(|_| "O resultado da instalação está indisponível. Confira o aparelho antes de repetir.".to_owned())?
                .map(Update::RunnerInstalled)
        });
    }

    /// Cancel immediately on UI lifecycle/config changes. Does not clear a list,
    /// alter foreground loading/error state, or restart anything on the phone.
    pub fn cancel_discovery(&mut self) {
        self.discovery_sequence = self.discovery_sequence.wrapping_add(1);
        self.discovery_job = None;
    }
    pub fn discovery_config_changed(&mut self, config: &Config) {
        if self.discovery_config.as_ref() != Some(config) {
            self.cancel_discovery();
            self.discovery_config = Some(config.clone());
            self.discovery_failed = false;
        }
    }
    /// One bounded, read-only background query. A connection stays clickable
    /// while this runs, and foreground operations cancel it before starting.
    pub fn search_automatically(&mut self, config: Config, cx: &mut Context<Self>) {
        self.discovery_config_changed(&config);
        if self.busy || self.session.is_some() || self.discovery_job.is_some() {
            return;
        }
        self.last_config = config.clone();
        self.discovery_sequence = self.discovery_sequence.wrapping_add(1);
        let sequence = self.discovery_sequence;
        let generation = self.generation;
        let query_config = config.clone();
        let worker = self.runtime.spawn(async move {
            tokio::time::timeout(Duration::from_secs(12), doctor(&query_config))
                .await
                .map_err(|_| "Automatic discovery timed out".to_owned())?
        });
        let abort = worker.abort_handle();
        let ui = cx.spawn(async move |this, cx| {
            let result = worker.await;
            // WeakEntity prevents keeping a closed model/window alive.
            let _ = this.update(cx, |model, cx| {
                if model.discovery_sequence != sequence
                    || model.generation != generation
                    || model.discovery_config.as_ref() != Some(&config)
                {
                    return;
                }
                model.discovery_job = None;
                if model.busy || model.session.is_some() {
                    return;
                }
                match result {
                    Ok(Ok(report)) => {
                        model.discovery_failed = false;
                        model.report = Some(report);
                    }
                    Ok(Err(_)) | Err(_) => model.discovery_failed = true,
                }
                // Quiet polling does not change status, error, busy or event history.
                cx.notify();
            });
        });
        self.discovery_job = Some(DiscoveryJob {
            worker: abort,
            _ui: ui,
        });
    }
    pub fn search(&mut self, config: Config, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if self.session.is_some() {
            self.diagnose(cx);
            return;
        }
        self.last_config = config.clone();
        self.perform("Buscando aparelho…", cx, async move {
            Ok(Update::Report(doctor(&config).await?))
        });
    }
    pub fn diagnose(&mut self, cx: &mut Context<Self>) {
        let session = self.session.clone();
        let config = self.last_config.clone();
        self.perform("Verificando conexão…", cx, async move {
            let report = if let Some(session) = session {
                session
                    .client
                    .diagnostics()
                    .await
                    .map_err(|e| e.to_string())?
            } else {
                doctor(&config).await?
            };
            Ok(Update::Report(report))
        });
    }
    pub fn connect(&mut self, device: DeviceSummary, config: Config, cx: &mut Context<Self>) {
        if self.session.is_some() || self.busy {
            return;
        }
        self.last_config = config.clone();
        self.element_provider = config.element_provider;
        self.elements = None;
        self.perform("Abrindo sessão…", cx, async move {
            let (session, report, capabilities) = start_session(device, config).await?;
            Ok(Update::Session(Arc::new(session), report, capabilities))
        });
    }
    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.cancel_discovery();
        self.generation = self.generation.wrapping_add(1);
        self.session = None;
        self.elements = None;
        self.capabilities = None;
        self.error = None;
        self.status = "Sessão encerrada".into();
        cx.notify();
    }
    pub fn set_element_provider(&mut self, provider: ElementProvider, cx: &mut Context<Self>) {
        if self.busy || self.element_provider == provider {
            return;
        }
        self.element_provider = provider;
        self.last_config.element_provider = provider;
        self.elements = None;
        cx.notify();
    }
    pub fn inspect(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        if self.busy {
            return;
        }
        let provider = self.element_provider;
        self.elements = None;
        self.perform("Inspecionando elementos…", cx, async move {
            session
                .client
                .ui_snapshot_with_provider(&session.device_id, provider)
                .await
                .map(Update::Elements)
                .map_err(|e| e.to_string())
        });
    }
    pub fn activate(&mut self, element_id: String, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let (Some(session), Some(snapshot)) = (self.session.clone(), self.elements.take()) else {
            return;
        };
        let Some(element) = snapshot
            .elements
            .iter()
            .find(|element| element.element_id == element_id)
        else {
            return;
        };
        if !snapshot.activation_supported
            || element.enabled == Some(false)
            || !element.actions.iter().any(|action| action == "activate")
        {
            return;
        }
        let request = UiActivateRequest {
            snapshot_id: snapshot.snapshot_id.clone(),
            element_id,
        };
        let provider = ElementProvider::Wda;
        self.perform("Ativando elemento…", cx, async move {
            session
                .client
                .activate_element(&session.device_id, &request)
                .await
                .map_err(|e| e.to_string())?;
            // Observation verifies the submitted action without replaying it.
            session
                .client
                .ui_snapshot_with_provider(&session.device_id, provider)
                .await
                .map(Update::Elements)
                .map_err(|e| e.to_string())
        });
    }
    pub fn recovery_remaining(&self) -> u64 {
        self.recovery_at
            .map(|at| 60u64.saturating_sub(at.elapsed().as_secs()))
            .unwrap_or(0)
    }
    fn reserve_recovery(&mut self, cx: &mut Context<Self>) -> bool {
        if self.busy {
            return false;
        }
        if self.recovery_remaining() > 0 {
            self.error = Some(format!(
                "Aguarde {} s antes de outra recuperação.",
                self.recovery_remaining()
            ));
            cx.notify();
            return false;
        }
        self.recovery_at = Some(Instant::now());
        self.elements = None;
        true
    }
    pub fn recover_service(&mut self, cx: &mut Context<Self>) {
        if self
            .session
            .as_ref()
            .is_some_and(|session| !session.allow_service_recovery)
        {
            self.error = Some("Habilite recuperação do serviço antes de abrir a sessão.".into());
            cx.notify();
            return;
        }
        if !self.reserve_recovery(cx) {
            return;
        }
        let session = self.session.clone();
        self.perform("Recuperando serviço Apple…", cx, async move {
            let result = if let Some(session) = session {
                session
                    .client
                    .recover_service()
                    .await
                    .map_err(|e| e.to_string())?
            } else {
                serde_json::from_slice::<RecoveryReport>(
                    &local_command(&["--recover-service".into()], 50, true).await?,
                )
                .map_err(|_| "Resposta de recuperação incompatível.".to_owned())?
            };
            let error = (result.outcome != "completed").then(|| {
                format!(
                    "{} · {}. Consulte os privilégios e o diagnóstico.",
                    result.code, result.outcome
                )
            });
            Ok(Update::Recovery(
                result.diagnostics,
                format!("Serviço: {} · {}", result.outcome, result.code),
                error,
            ))
        });
    }
    pub fn recover_runner(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        if !session.runner_configured {
            self.error = Some("Esta sessão não tem um runner configurado.".into());
            cx.notify();
            return;
        }
        if !self.reserve_recovery(cx) {
            return;
        }
        self.perform("Recuperando automação…", cx, async move {
            session
                .client
                .recover_elements()
                .await
                .map_err(|e| e.to_string())?;
            Ok(Update::Message(
                "Runner reiniciado. Faça uma nova inspeção.".into(),
            ))
        });
    }
    pub fn recover_usb(&mut self, instance: String, cx: &mut Context<Self>) {
        let Some(target) = self
            .report
            .as_ref()
            .and_then(|report| {
                report
                    .usb_recovery_targets
                    .iter()
                    .find(|target| target.instance_id == instance)
            })
            .cloned()
        else {
            self.error = Some("Atualize o diagnóstico e selecione uma interface atual.".into());
            cx.notify();
            return;
        };
        if self.session.as_ref().is_some_and(|session| {
            !session.allow_usb_recovery || session.device_id != target.device_id
        }) {
            self.error =
                Some("A recuperação USB não foi habilitada para este aparelho na sessão.".into());
            cx.notify();
            return;
        }
        if !self.reserve_recovery(cx) {
            return;
        }
        let session = self.session.clone();
        self.perform("Recuperando interface USB…", cx, async move {
            let result = if let Some(session) = session {
                session.client.recover_usb_interface(&UsbRecoveryRequest { device_id: target.device_id, instance_id: instance }).await.map_err(|e| e.to_string())?
            } else { serde_json::from_slice::<UsbRecoveryReport>(&local_command(&["--device".into(),target.device_id,"--recover-usb-interface".into(),instance],45,true).await?).map_err(|_| "Resposta USB incompatível.".to_owned())? };
            let error = (result.outcome != "completed").then(|| if result.outcome == "reboot_required" {
                "O Windows exige reinicialização. Salve seu trabalho; o programa não reinicia o computador.".into()
            } else { format!("{} · {}. Consulte o diagnóstico antes de repetir.",result.code,result.outcome) });
            Ok(Update::Recovery(result.diagnostics,format!("Interface USB: {}",result.outcome),error))
        });
    }
    pub fn enable_wifi(&mut self, udid: String, cx: &mut Context<Self>) {
        self.perform("Preparando Wi-Fi pelo USB pareado…", cx, async move {
            rust_ios_device_client::validate_device_id(&udid).map_err(|e| e.to_string())?;
            let bytes = local_command(
                &["--device".into(), udid, "--enable-wifi".into()],
                20,
                false,
            )
            .await?;
            let result: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|_| "Resposta de preparação incompatível.".to_owned())?;
            if result.get("code").and_then(|value| value.as_str()) == Some("wifi_setting_enabled") {
                Ok(Update::Message(
                    "Wi-Fi habilitado. Retire o cabo e busque pela rede.".into(),
                ))
            } else {
                Err(result
                    .get("message")
                    .and_then(|value| value.as_str())
                    .unwrap_or("Confira o pareamento USB e o diagnóstico.")
                    .into())
            }
        });
    }
    pub fn connect_existing(&mut self, origin: String, token: String, cx: &mut Context<Self>) {
        if self.session.is_some() {
            return;
        }
        self.perform("Conectando ao servidor…",cx,async move {
            let client=DeviceClient::new(&origin,&token).map_err(|e|e.to_string())?;
            client.health().await.map_err(|e|e.to_string())?;
            let report=client.diagnostics().await.map_err(|e|e.to_string())?;
            let devices:std::collections::BTreeMap<_,_>=report.devices.iter().filter(|device|device.identity_status==IdentityStatus::Available).map(|device|(&device.device_id,device)).collect();
            if devices.len()!=1 { return Err("O servidor precisa oferecer exatamente um aparelho com identidade disponível para abrir esta sessão.".into()); }
            let device=devices.values().next().ok_or("Nenhum aparelho disponível.")?;
            let capabilities=client.capabilities().await.map_err(|e|e.to_string())?;
            let session=Session {client,device_id:device.device_id.clone(),name:device.name.clone().unwrap_or_else(||"iPhone".into()),product_type:device.product_type.clone(),origin,token,transport:Transport::Rsd,
                runner_configured:capabilities.pointer("/features/ui_elements/runner_start_allowed").or_else(||capabilities.pointer("/features/ui_elements/runner_configured")).and_then(|value|value.as_bool()).unwrap_or(false),
                allow_usb_recovery:capabilities.get("usb_interface_restart").and_then(|value|value.as_bool()).unwrap_or(false),
                allow_service_recovery:capabilities.pointer("/features/service_restart").and_then(|value|value.as_bool()).unwrap_or(false),_child:None};
            Ok(Update::Session(Arc::new(session),report,capabilities))
        });
    }
    pub fn mcp_config(&self) -> Result<String, String> {
        let session = self.session.as_ref().ok_or("Abra uma sessão primeiro.")?;
        let command = sibling("rust-ios-device-mcp")?;
        Ok(serde_json::json!({"mcpServers":{"tetherdeck":{"command":command,"env":{"RUST_IOS_DEVICE_URL":session.origin,"RUST_IOS_DEVICE_TOKEN":session.token}}}}).to_string())
    }
}

fn sibling(name: &str) -> Result<PathBuf, String> {
    let mut path =
        std::env::current_exe().map_err(|_| "Não foi possível localizar o programa.".to_owned())?;
    path.set_file_name(format!("{name}.exe"));
    Ok(path)
}
async fn local_command(args: &[String], seconds: u64, recovery: bool) -> Result<Vec<u8>, String> {
    let mut command = tokio::process::Command::new(sibling("rust-ios-device-daemon")?);
    command.args(args).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output=tokio::time::timeout(Duration::from_secs(seconds),command.output()).await
        .map_err(|_|if recovery {"A recuperação excedeu o prazo e pode ter resultado incerto. Consulte o diagnóstico antes de repetir.".to_owned()}else{"O diagnóstico excedeu o prazo.".to_owned()})?
        .map_err(|_|"Não foi possível executar o daemon. Mantenha os executáveis na mesma pasta.".to_owned())?;
    if output.stdout.len() > 1024 * 1024 {
        return Err("A resposta excedeu o limite.".into());
    }
    if !output.status.success() && !recovery {
        return Err(
            "O daemon não concluiu a operação. Consulte o diagnóstico e o pareamento.".into(),
        );
    }
    Ok(output.stdout)
}
async fn doctor(config: &Config) -> Result<DiagnosticReport, String> {
    let mut args = vec![
        "--doctor".into(),
        "--transport".into(),
        if config.transport == Transport::Wifi {
            "wifi".into()
        } else {
            "usb".into()
        },
    ];
    if config.transport != Transport::Usb {
        rust_ios_device_client::validate_device_id(&config.udid).map_err(|e| e.to_string())?;
        args.extend(["--device".into(), config.udid.clone()]);
        if config.transport == Transport::Wifi {
            args.push("--wifi-direct".into());
            if !config.host.trim().is_empty() {
                args.extend(["--wifi-host".into(), config.host.trim().into()]);
            }
        } else {
            rust_ios_device_client::validate_origin(&config.tunneld_origin)
                .map_err(|e| e.to_string())?;
            args.extend(["--tunneld-url".into(), config.tunneld_origin.clone()]);
        }
    }
    serde_json::from_slice(&local_command(&args, 30, false).await?)
        .map_err(|_| "Resposta de diagnóstico incompatível.".into())
}
async fn start_session(
    device: DeviceSummary,
    config: Config,
) -> Result<(Session, DiagnosticReport, serde_json::Value), String> {
    let id = device.udid.clone().unwrap_or(device.device_id.clone());
    rust_ios_device_client::validate_device_id(&id).map_err(|e| e.to_string())?;
    if !config.runner_bundle.is_empty()
        && (config.runner_bundle.len() > 255
            || !config.runner_bundle.contains('.')
            || !config
                .runner_bundle
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'))
    {
        return Err("Bundle ID do runner inválido.".into());
    }
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .map_err(|_| "Não foi possível reservar a porta local.".to_owned())?;
    let address = listener
        .local_addr()
        .map_err(|_| "Porta local indisponível.".to_owned())?;
    drop(listener);
    let origin = format!("http://{address}");
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let mut command = tokio::process::Command::new(sibling("rust-ios-device-daemon")?);
    command
        .args(["--listen", &address.to_string(), "--device", &id])
        .env("RUST_IOS_DEVICE_TOKEN", &token)
        .env("RUST_LOG", "warn")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    if config.allow_usb_recovery {
        command.arg("--allow-usb-interface-restart");
    }
    if config.allow_service_recovery {
        command.arg("--allow-service-restart");
    }
    match config.transport {
        Transport::Wifi => {
            if !config.runner_bundle.is_empty() {
                command.args(["--wda-runner", &config.runner_bundle]);
                if config.runner_tunnel_binding_reviewed_bundle == config.runner_bundle {
                    command.arg("--wda-tunnel-binding-reviewed");
                }
            }
            command.args(["--transport", "wifi", "--wifi-direct"]);
            let host = device
                .transport
                .strip_prefix("network:")
                .unwrap_or(config.host.trim());
            if !host.is_empty() {
                command.args(["--wifi-host", host]);
            }
        }
        Transport::Rsd => {
            rust_ios_device_client::validate_origin(&config.tunneld_origin)
                .map_err(|e| e.to_string())?;
            command.args(["--tunneld-url", &config.tunneld_origin]);
        }
        Transport::Usb => {
            command.args(["--transport", "usb"]);
            if !config.runner_bundle.is_empty() {
                command.args(["--wda-runner", &config.runner_bundle]);
            }
        }
    }
    let mut child = command
        .spawn()
        .map_err(|_| "Não foi possível iniciar o daemon local.".to_owned())?;
    let client = DeviceClient::new(&origin, &token).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(150);
    loop {
        if child
            .try_wait()
            .map_err(|_| "Não foi possível consultar o daemon.".to_owned())?
            .is_some()
        {
            return Err("O daemon encerrou durante a conexão. Confira pareamento, transporte e opções da sessão.".into());
        }
        if matches!(
            tokio::time::timeout(Duration::from_secs(1), client.health()).await,
            Ok(Ok(_))
        ) {
            break;
        }
        if Instant::now() >= deadline {
            return Err("A conexão excedeu o prazo. Consulte o diagnóstico.".into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let report = client.diagnostics().await.map_err(|e| e.to_string())?;
    let verified = report.devices.iter().any(|device| {
        (device.device_id == id || device.udid.as_deref() == Some(id.as_str()))
            && device.identity_status == IdentityStatus::Available
            && match config.transport {
                Transport::Usb => device.transport == "usb",
                Transport::Wifi => device.transport.starts_with("network:"),
                Transport::Rsd => device.transport.starts_with("rsd:"),
            }
    });
    if !verified {
        return Err("O daemon iniciou, mas não confirmou a identidade do iPhone no transporte escolhido. Desbloqueie o aparelho e atualize o diagnóstico.".into());
    }
    let capabilities = client.capabilities().await.map_err(|e| e.to_string())?;
    let session = Session {
        client,
        device_id: id,
        name: device.name.unwrap_or_else(|| "iPhone".into()),
        product_type: device.product_type,
        origin,
        token,
        transport: config.transport,
        runner_configured: config.transport != Transport::Rsd
            && !config.runner_bundle.is_empty()
            && (config.transport != Transport::Wifi
                || config.runner_tunnel_binding_reviewed_bundle == config.runner_bundle),
        allow_usb_recovery: config.allow_usb_recovery,
        allow_service_recovery: config.allow_service_recovery,
        _child: Some(child),
    };
    Ok((session, report, capabilities))
}

fn known_wifi_device() -> String {
    let Some(home) = std::env::var_os("USERPROFILE") else {
        return String::new();
    };
    let Ok(entries) = std::fs::read_dir(PathBuf::from(home).join(".pymobiledevice3")) else {
        return String::new();
    };
    let ids: Vec<_> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let id = name
                .to_str()?
                .strip_prefix("remote_")?
                .strip_suffix(".plist")?;
            let normalized = id.replace('-', "");
            (matches!(normalized.len(), 24 | 40)
                && normalized.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| id.to_owned())
        })
        .collect();
    if ids.len() == 1 {
        ids[0].clone()
    } else {
        String::new()
    }
}
