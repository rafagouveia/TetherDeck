//! Native WDA control with an AX inspection fallback. No screenshot/OCR is used.
use crate::{DeviceError, accessibility::AxSession};
use idevice::{
    IdeviceService,
    provider::{IdeviceProvider, RsdProvider, UsbmuxdProvider},
    services::{
        dvt::xctest::{TestConfig, WdaRunHandle, XCUITestService},
        installation_proxy::InstallationProxyClient,
        wda::{WdaClient, WdaPorts},
    },
    usbmuxd::{Connection, UsbmuxdAddr},
};
use rust_ios_device_api::{ElementProvider, UiElement, UiRect, UiSnapshot};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

async fn mux_provider(id: &str) -> Result<UsbmuxdProvider, DeviceError> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut mux = UsbmuxdAddr::from_env_var()?
            .connect(0)
            .await
            .map_err(|error| wda_error("mux_connect", error))?;
        let device =
            crate::wifi::selected_device(&mut mux, id)
                .await
                .map_err(|error| match error {
                    idevice::IdeviceError::DeviceNotFound => DeviceError::MissingDevice(id.into()),
                    other => wda_error("mux_enumeration", other),
                })?;
        // A Network listing alone does not establish an authenticated HTTP channel.
        // Use the dedicated paired TLS/RSD path instead of raw Wi-Fi WDA forwarding.
        if !matches!(device.connection_type, Connection::Usb) {
            return Err(DeviceError::WdaNetworkTunnelRequired);
        }
        super::provider(device.device_id, id.into())
    })
    .await
    .map_err(|_| DeviceError::Timeout)?
}
// Only an explicitly started runner owns a paired tunnel. No LAN HTTP fallback.
static WIFI_WDA: OnceLock<Mutex<HashMap<String, Weak<WdaTransport>>>> = OnceLock::new();
#[derive(Clone)]
enum WdaTransport {
    Mux(UsbmuxdProvider),
    Tunnel {
        base: Arc<dyn IdeviceProvider>,
        adapter: idevice::tcp::handle::AdapterHandle,
        handshake: idevice::services::rsd::RsdHandshake,
        port: u16,
        generation: String,
    },
}
impl std::fmt::Debug for WdaTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Mux(_) => "WdaTransport::Mux",
            Self::Tunnel { .. } => "WdaTransport::PairedTunnel",
        })
    }
}
impl WdaTransport {
    fn identity(&self) -> String {
        match self {
            Self::Mux(p) => format!("mux:{}", p.device_id),
            Self::Tunnel { generation, .. } => generation.clone(),
        }
    }
}
impl IdeviceProvider for WdaTransport {
    fn label(&self) -> &str {
        "tetherdeck-wda-paired"
    }
    fn get_pairing_file(
        &self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<idevice::pairing_file::PairingFile, idevice::IdeviceError>,
                > + Send,
        >,
    > {
        match self {
            Self::Mux(p) => p.get_pairing_file(),
            Self::Tunnel { base, .. } => base.get_pairing_file(),
        }
    }
    fn connect(
        &self,
        port: u16,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<idevice::Idevice, idevice::IdeviceError>>
                + Send,
        >,
    > {
        let transport = self.clone();
        Box::pin(async move {
            match &transport {
                Self::Mux(p) => p.connect(port).await,
                Self::Tunnel {
                    adapter,
                    port: http,
                    ..
                } if port == *http => {
                    let stream = adapter.clone().connect_to_service_port(port).await?;
                    Ok(idevice::Idevice::new(stream, "tetherdeck-wda-paired"))
                }
                Self::Tunnel { base, .. } => base.connect(port).await,
            }
        })
    }
}
async fn provider(id: &str) -> Result<Arc<WdaTransport>, DeviceError> {
    if crate::wireless::id().is_some() {
        if crate::wireless::id() != Some(id) {
            return Err(DeviceError::DeviceIdMismatch);
        }
        return WIFI_WDA
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| DeviceError::WdaUnavailable)?
            .get(id)
            .and_then(Weak::upgrade)
            .ok_or(DeviceError::WdaUnavailable);
    }
    Ok(Arc::new(WdaTransport::Mux(mux_provider(id).await?)))
}
pub struct Runner {
    handle: WdaRunHandle,
    _transport: Arc<WdaTransport>,
}
impl Drop for Runner {
    fn drop(&mut self) {
        self.handle.abort();
    }
}
pub async fn start_runner(id: &str, bundle: &str, port: u16) -> Result<Runner, DeviceError> {
    start_runner_with_binding_review(id, bundle, port, false).await
}
/// A review applies to the exact installed runner selected by the scoped caller.
/// It is a compatibility declaration, not binary attestation or a TLS guarantee for arbitrary IPAs.
pub async fn start_runner_with_binding_review(
    id: &str,
    bundle: &str,
    port: u16,
    binding_reviewed: bool,
) -> Result<Runner, DeviceError> {
    let direct = crate::wireless::id().is_some();
    if direct && !binding_reviewed {
        return Err(DeviceError::WdaTunnelBindingReviewRequired);
    }
    let base: Arc<dyn IdeviceProvider> = if direct {
        Arc::from(crate::wireless::provider(id).await?)
    } else {
        Arc::new(mux_provider(id).await?)
    };
    let mut stack = if direct {
        Some(
            tokio::time::timeout(Duration::from_secs(20), super::usb_rsd_stack(id.into()))
                .await
                .map_err(|_| DeviceError::Timeout)??,
        )
    } else {
        None
    };
    let mut config = tokio::time::timeout(Duration::from_secs(20), async {
        let mut installer = if let Some((adapter, handshake)) = stack.as_mut() {
            // Remote InstallationProxy is a plist shim, not raw Lockdown LAN traffic.
            // https://idevice.wiki/docs/service-reference/files-apps/installation-proxy/
            let service = handshake
                .services
                .get("com.apple.mobile.installation_proxy.shim.remote")
                .ok_or_else(|| {
                    wda_error(
                        "runner_metadata_service",
                        idevice::IdeviceError::ServiceNotFound,
                    )
                })?;
            let stream = adapter
                .connect_to_service_port(service.port)
                .await
                .map_err(|e| wda_error("runner_metadata_connect", e))?;
            let mut device = idevice::Idevice::new(stream, "tetherdeck-runner-metadata");
            device
                .rsd_checkin()
                .await
                .map_err(|e| wda_error("runner_metadata_checkin", e))?;
            InstallationProxyClient::new(device)
        } else {
            InstallationProxyClient::connect(base.as_ref())
                .await
                .map_err(|e| wda_error("runner_lockdown", e))?
        };
        runner_configuration(&mut installer, bundle).await
    })
    .await
    .map_err(|_| DeviceError::Timeout)??;
    // Stop only this installed runner. Aborting the host DTX task alone can leave WDA alive.
    let stop_id = id.to_owned();
    let stop_config = config.clone();
    let stop_stack = stack.clone();
    tokio::task::spawn_blocking(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(DeviceError::RsdTransport)?;
        runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(15),
                stop_configured_runner(&stop_id, &stop_config, stop_stack),
            )
            .await
            .map_err(|_| DeviceError::Timeout)?
        })
    })
    .await
    .map_err(|_| DeviceError::ActionOutcomeUnknown)??;
    let (transport, binding) = if let Some((adapter, handshake)) = stack {
        // Address is inside the mutually authenticated RemotePairing tunnel, never a LAN bind.
        let binding = adapter.peer_ip();
        if !matches!(binding, std::net::IpAddr::V6(ip) if (ip.segments()[0] & 0xfe00) == 0xfc00) {
            return Err(DeviceError::InvalidRsdAddress);
        }
        (
            Arc::new(WdaTransport::Tunnel {
                base,
                adapter,
                handshake,
                port,
                generation: uuid::Uuid::new_v4().to_string(),
            }),
            binding.to_string(),
        )
    } else {
        (
            Arc::new(WdaTransport::Mux(mux_provider(id).await?)),
            "127.0.0.1".into(),
        )
    };
    config.runner_env = Some(plist::Dictionary::from_iter([
        ("USE_PORT", plist::Value::String(port.to_string())),
        ("USE_IP", plist::Value::String(binding)),
        (
            "UPGRADE_TIMESTAMP",
            plist::Value::String(uuid::Uuid::new_v4().to_string()),
        ),
        (
            "WDA_PRODUCT_BUNDLE_IDENTIFIER",
            plist::Value::String(bundle.to_owned()),
        ),
    ]));
    let mut service = XCUITestService::new(transport.clone());
    if let WdaTransport::Tunnel {
        adapter, handshake, ..
    } = transport.as_ref()
    {
        service = service.with_rsd_stack(adapter.clone(), handshake.clone());
    }
    let handle = service
        .run_until_wda_ready(config, Duration::from_secs(90))
        .await?;
    if direct {
        WIFI_WDA
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| DeviceError::WdaUnavailable)?
            .insert(id.into(), Arc::downgrade(&transport));
    }
    Ok(Runner {
        handle,
        _transport: transport,
    })
}
async fn runner_configuration(
    installer: &mut InstallationProxyClient,
    bundle: &str,
) -> Result<TestConfig, DeviceError> {
    let apps = installer
        .get_apps(None, Some(vec![bundle.to_owned()]))
        .await
        .map_err(|e| wda_error("runner_lookup", e))?;
    let info = apps
        .get(bundle)
        .and_then(plist::Value::as_dictionary)
        .ok_or_else(|| {
            wda_error(
                "runner_not_installed",
                idevice::IdeviceError::AppNotInstalled,
            )
        })?;
    let field = |key: &str, stage: &'static str| -> Result<String, DeviceError> {
        info.get(key)
            .and_then(plist::Value::as_string)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                wda_error(
                    stage,
                    idevice::IdeviceError::UnexpectedResponse(
                        "required runner metadata is missing".into(),
                    ),
                )
            })
    };
    let executable = field("CFBundleExecutable", "runner_metadata_executable")?;
    if !executable.ends_with("-Runner") {
        return Err(wda_error(
            "runner_metadata_executable",
            idevice::IdeviceError::UnexpectedResponse(
                "runner executable has unsupported format".into(),
            ),
        ));
    }
    Ok(TestConfig {
        runner_bundle_id: bundle.into(),
        runner_app_path: field("Path", "runner_metadata_path")?,
        runner_app_container: field("Container", "runner_metadata_container")?,
        runner_bundle_executable: executable,
        target_bundle_id: None,
        target_app_path: None,
        target_app_env: None,
        target_app_args: None,
        tests_to_run: None,
        tests_to_skip: None,
        runner_env: None,
        runner_args: None,
    })
}
/// Fixed metadata only, safe for public diagnostics; never protocol payloads or paths.
pub fn startup_failure(error: &DeviceError) -> (&'static str, &'static str) {
    let (stage, cause) = match error {
        DeviceError::ElementProviderFailed { stage, source } => (*stage, Some(source)),
        DeviceError::WdaTunnelBindingReviewRequired => return ("binding_review", "required"),
        DeviceError::WdaNetworkTunnelRequired => return ("transport", "paired_tunnel_required"),
        DeviceError::Timeout => return ("startup", "timeout"),
        DeviceError::WifiPairingRequired => return ("transport", "pairing_required"),
        DeviceError::DeviceIdMismatch => return ("transport", "device_mismatch"),
        DeviceError::Idevice(source) => ("xctest", Some(source)),
        _ => ("startup", None),
    };
    let kind = match cause {
        Some(idevice::IdeviceError::AppNotInstalled) => "not_installed",
        Some(idevice::IdeviceError::ServiceNotFound) => "service_not_advertised",
        Some(idevice::IdeviceError::DeviceLocked) => "device_locked",
        Some(idevice::IdeviceError::DeveloperModeNotEnabled) => "developer_mode_required",
        Some(idevice::IdeviceError::ImageNotMounted) => "ddi_required",
        Some(idevice::IdeviceError::Timeout | idevice::IdeviceError::TestRunnerTimeout) => {
            "timeout"
        }
        Some(idevice::IdeviceError::TestRunnerDisconnected) => "runner_disconnected",
        Some(idevice::IdeviceError::UnexpectedResponse(_) | idevice::IdeviceError::Plist(_)) => {
            "protocol"
        }
        Some(idevice::IdeviceError::Socket(error))
            if error.kind() == std::io::ErrorKind::UnexpectedEof =>
        {
            "connection_closed"
        }
        Some(idevice::IdeviceError::Socket(_)) => "transport_io",
        _ => "unavailable",
    };
    (stage, kind)
}
async fn stop_configured_runner(
    id: &str,
    config: &TestConfig,
    stack: Option<(
        idevice::tcp::handle::AdapterHandle,
        idevice::services::rsd::RsdHandshake,
    )>,
) -> Result<(), DeviceError> {
    let (mut adapter, mut handshake) = match stack {
        Some(stack) => stack,
        None => super::usb_rsd_stack(id.into()).await?,
    };
    let mut app: idevice::services::core_device::AppServiceClient<Box<dyn idevice::ReadWrite>> =
        handshake.connect(&mut adapter).await?;
    let expected = format!(
        "{}/{}",
        config.runner_app_path.trim_end_matches('/'),
        config.runner_bundle_executable
    );
    let processes = app.list_processes().await?;
    let matches: Vec<_> = processes
        .into_iter()
        .filter(|p| {
            p.executable_url
                .as_ref()
                .is_some_and(|u| u.relative.trim_start_matches("file://") == expected)
        })
        .collect();
    if matches.len() > 1 {
        return Err(DeviceError::WdaUnavailable);
    }
    if let Some(process) = matches.first() {
        app.send_signal(process.pid, 15).await?;
        for _ in 0..10 {
            if !app
                .list_processes()
                .await?
                .iter()
                .any(|p| p.pid == process.pid)
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        return Err(DeviceError::WdaUnavailable);
    }
    Ok(())
}
pub enum ElementSession {
    Wda(WdaSession),
    Ax(AxSession),
}
impl ElementSession {
    pub fn invalidate(&mut self) {
        if let Self::Wda(session) = self {
            session.references.clear();
            session.generation.clear();
        }
    }
    pub fn provider_name(&self) -> &'static str {
        match self {
            Self::Wda(_) => "wda",
            Self::Ax(_) => "ios_ax",
        }
    }
    pub async fn connect(id: &str, port: u16, require_wda: bool) -> Result<Self, DeviceError> {
        Self::connect_with_provider(id, port, require_wda, ElementProvider::Auto).await
    }
    pub async fn connect_with_provider(
        id: &str,
        port: u16,
        require_wda: bool,
        selected: ElementProvider,
    ) -> Result<Self, DeviceError> {
        if selected == ElementProvider::IosAx {
            if require_wda {
                return Err(DeviceError::WdaUnavailable);
            }
            return Ok(Self::Ax(AxSession::connect(id.into()).await?));
        }
        let candidate = match provider(id).await {
            Ok(provider) => Some(provider),
            Err(error) if require_wda || selected == ElementProvider::Wda => return Err(error),
            Err(_) => None,
        };
        let ready = if let Some(provider) = candidate {
            client(provider.as_ref(), port)
                .with_timeout(Duration::from_secs(2))
                .status()
                .await
                .is_ok_and(|status| {
                    status.pointer("/value/ready").and_then(Value::as_bool) == Some(true)
                })
        } else {
            false
        };
        if ready {
            return Ok(Self::Wda(WdaSession::connect(id, port).await?));
        }
        if require_wda || selected == ElementProvider::Wda {
            return Err(DeviceError::WdaUnavailable);
        }
        Ok(Self::Ax(AxSession::connect(id.into()).await?))
    }
    pub async fn snapshot(&mut self) -> Result<UiSnapshot, DeviceError> {
        match self {
            Self::Wda(s) => {
                let result = s.snapshot().await;
                if matches!(&result, Err(DeviceError::ElementSessionLost)) {
                    *s = WdaSession::connect(&s.id, s.port).await?;
                    return s.snapshot().await;
                }
                result
            }
            Self::Ax(s) => s.snapshot().await,
        }
    }
    pub async fn activate(&mut self, snapshot: &str, element: &str) -> Result<(), DeviceError> {
        match self {
            Self::Wda(s) => s.activate(snapshot, element).await,
            Self::Ax(_) => Err(DeviceError::ElementActivationUnsupported),
        }
    }
}
pub async fn probe_wda(id: &str, port: u16) -> Result<(), DeviceError> {
    let provider = provider(id).await?;
    client(provider.as_ref(), port)
        .with_timeout(Duration::from_secs(2))
        .status()
        .await
        .and_then(|status| {
            if status.pointer("/value/ready").and_then(Value::as_bool) == Some(true) {
                Ok(())
            } else {
                Err(idevice::IdeviceError::UnexpectedResponse(
                    "WDA is not ready".into(),
                ))
            }
        })
        .map_err(|_| DeviceError::WdaUnavailable)
}
fn client(provider: &dyn IdeviceProvider, port: u16) -> WdaClient<'_> {
    WdaClient::new(provider).with_ports(WdaPorts {
        http: port,
        mjpeg: 9100,
    })
}
#[derive(PartialEq, Eq)]
struct AppIdentity {
    bundle: String,
    pid: u64,
}
async fn app_identity(client: &WdaClient<'_>, session: &str) -> Result<AppIdentity, DeviceError> {
    let info = client
        .active_app_info(session)
        .await
        .map_err(|e| wda_error("app_identity", e))?;
    let bundle = info
        .get("bundleId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or(DeviceError::WdaUnavailable)?
        .to_owned();
    let pid = info
        .get("pid")
        .and_then(Value::as_u64)
        .filter(|p| *p > 0)
        .ok_or(DeviceError::WdaUnavailable)?;
    Ok(AppIdentity { bundle, pid })
}
struct Reference {
    activatable: bool,
    xpath: String,
    name: String,
    label: String,
    role: String,
    value: Option<String>,
}
pub struct WdaSession {
    transport_id: String,
    id: String,
    session: String,
    port: u16,
    generation: String,
    at: Instant,
    app: Option<AppIdentity>,
    references: HashMap<String, Reference>,
}
impl WdaSession {
    async fn connect(id: &str, port: u16) -> Result<Self, DeviceError> {
        let provider = provider(id).await?;
        let session = client(provider.as_ref(), port)
            .start_session(None)
            .await
            .map_err(|e| wda_error("session_create", e))?;
        Ok(Self {
            transport_id: provider.identity(),
            id: id.into(),
            session,
            port,
            generation: String::new(),
            at: Instant::now(),
            app: None,
            references: HashMap::new(),
        })
    }
    async fn snapshot(&mut self) -> Result<UiSnapshot, DeviceError> {
        self.references.clear();
        self.generation.clear();
        self.app = None;
        let provider = provider(&self.id).await?;
        let client = client(provider.as_ref(), self.port);
        let app = app_identity(&client, &self.session).await?;
        // JSON source preserves rawIdentifier, hierarchy and state without inferring IDs from labels.
        let source = client
            .source_json(&self.session)
            .await
            .map_err(|e| wda_error("source_query", e))?;
        let mut elements = Vec::new();
        let mut references = HashMap::new();
        let mut complete = true;
        walk(
            &source,
            "",
            None,
            0,
            &mut elements,
            &mut references,
            &mut complete,
        )?;
        if elements.is_empty() {
            return Err(DeviceError::WdaUnavailable);
        }
        if app_identity(&client, &self.session).await? != app {
            return Err(DeviceError::StaleElement);
        }
        self.references = references;
        self.transport_id = provider.identity();
        self.generation = uuid::Uuid::new_v4().to_string();
        self.at = Instant::now();
        let bundle = app.bundle.clone();
        self.app = Some(app);
        Ok(UiSnapshot {
            app_bundle_id: Some(bundle),
            bounds_units: Some("points".into()),
            activation_supported: true,
            device_id: self.id.clone(),
            snapshot_id: self.generation.clone(),
            provider: "wda".into(),
            created_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            complete,
            elements,
            warnings: if complete {
                vec![]
            } else {
                vec!["Árvore limitada a 2000 elementos ou 64 níveis. A completude depende do snapshot exposto pelo XCTest.".into()]
            },
        })
    }
    async fn activate(&mut self, generation: &str, element: &str) -> Result<(), DeviceError> {
        if generation != self.generation || self.at.elapsed() > Duration::from_secs(30) {
            return Err(DeviceError::StaleElement);
        }
        let reference = self
            .references
            .remove(element)
            .ok_or(DeviceError::StaleElement)?;
        // Consume references before any operation: no automatic replay after uncertainty.
        self.references.clear();
        self.generation.clear();
        let provider = provider(&self.id).await?;
        if !reference.activatable {
            return Err(DeviceError::ElementNotActionable);
        }
        let client = client(provider.as_ref(), self.port);
        if provider.identity() != self.transport_id {
            return Err(DeviceError::StaleElement);
        }
        if Some(app_identity(&client, &self.session).await?) != self.app {
            return Err(DeviceError::StaleElement);
        }
        let found = client
            .find_elements("xpath", &reference.xpath, Some(&self.session))
            .await
            .map_err(|e| wda_error("element_resolve", e))?;
        if found.len() != 1 {
            return Err(DeviceError::StaleElement);
        }
        for (attribute, expected) in [
            ("name", Some(reference.name)),
            ("label", Some(reference.label)),
            ("type", Some(reference.role)),
            ("value", reference.value),
        ] {
            let actual = client
                .element_attribute(&found[0], attribute, Some(&self.session))
                .await
                .map_err(|e| wda_error("element_validate", e))?;
            if actual.as_str().unwrap_or("") != expected.as_deref().unwrap_or("") {
                return Err(DeviceError::StaleElement);
            }
        }
        if !client
            .element_displayed(&found[0], Some(&self.session))
            .await
            .map_err(|e| wda_error("element_validate", e))?
        {
            return Err(DeviceError::StaleElement);
        }
        let enabled = client
            .element_attribute(&found[0], "enabled", Some(&self.session))
            .await
            .map_err(|e| wda_error("element_validate", e))?;
        if boolean(&enabled) != Some(true)
            || self.at.elapsed() > Duration::from_secs(30)
            || Some(app_identity(&client, &self.session).await?) != self.app
        {
            return Err(DeviceError::StaleElement);
        }
        client
            .click(&found[0], Some(&self.session))
            .await
            .map_err(|_| DeviceError::ActionOutcomeUnknown)?;
        Ok(())
    }
}
fn boolean(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(v) => Some(*v),
        Value::String(v) => match v.as_str() {
            "1" | "true" => Some(true),
            "0" | "false" => Some(false),
            _ => None,
        },
        Value::Number(v) => v.as_u64().and_then(|n| match n {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }),
        _ => None,
    }
}
fn wda_error(stage: &'static str, source: idevice::IdeviceError) -> DeviceError {
    if let idevice::IdeviceError::UnknownErrorType(message) = &source {
        let message = message.to_ascii_lowercase();
        if message.contains("invalid session id") || message.contains("session does not exist") {
            return DeviceError::ElementSessionLost;
        }
    }
    DeviceError::ElementProviderFailed { stage, source }
}
fn text(node: &Value, key: &str) -> String {
    node.get(key).and_then(Value::as_str).unwrap_or("").into()
}
fn node_role(node: &Value) -> Result<String, DeviceError> {
    let value = text(node, "type");
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(DeviceError::WdaUnavailable);
    }
    Ok(if value.starts_with("XCUIElementType") {
        value
    } else {
        format!("XCUIElementType{value}")
    })
}
fn walk(
    node: &Value,
    path: &str,
    parent: Option<&str>,
    depth: u32,
    elements: &mut Vec<UiElement>,
    references: &mut HashMap<String, Reference>,
    complete: &mut bool,
) -> Result<(), DeviceError> {
    if depth > 64 || elements.len() >= 2000 {
        *complete = false;
        return Ok(());
    }
    let role = node_role(node)?;
    let activatable = matches!(
        role.as_str(),
        "XCUIElementTypeButton"
            | "XCUIElementTypeLink"
            | "XCUIElementTypeCell"
            | "XCUIElementTypeSwitch"
            | "XCUIElementTypeSearchField"
            | "XCUIElementTypeTextField"
            | "XCUIElementTypeSecureTextField"
            | "XCUIElementTypeSegment"
            | "XCUIElementTypeSlider"
            | "XCUIElementTypePickerWheel"
    ) || text(node, "traits")
        .split(',')
        .any(|trait_name| matches!(trait_name.trim(), "Button" | "Link"));
    let path = if path.is_empty() {
        format!("//{role}[1]")
    } else {
        path.into()
    };
    let element_id = uuid::Uuid::new_v4().to_string();
    let name = text(node, "name");
    let label = text(node, "label");
    let value = node.get("value").filter(|v| !v.is_null()).map(|v| {
        v.as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| v.to_string())
    });
    let number = |key| {
        node.get("rect")
            .and_then(|r| r.get(key))
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite())
    };
    let bounds = match (number("x"), number("y"), number("width"), number("height")) {
        (Some(x), Some(y), Some(width), Some(height)) => Some(UiRect {
            x,
            y,
            width,
            height,
        }),
        _ => None,
    };
    references.insert(
        element_id.clone(),
        Reference {
            activatable,
            xpath: path.clone(),
            name,
            label: label.clone(),
            role: role.clone(),
            value: value.clone(),
        },
    );
    elements.push(UiElement {
        actions: if activatable {
            vec!["activate".into()]
        } else {
            vec![]
        },
        parent_id: parent.map(str::to_owned),
        depth,
        visible: node.get("isVisible").and_then(boolean),
        value,
        element_id: element_id.clone(),
        identifier: node
            .get("rawIdentifier")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_owned),
        label,
        role: Some(role),
        enabled: node.get("isEnabled").and_then(boolean),
        bounds,
    });
    let mut siblings = HashMap::<String, usize>::new();
    if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
            let role = node_role(child)?;
            let index = siblings.entry(role.clone()).or_default();
            *index += 1;
            walk(
                child,
                &format!("{path}/{role}[{index}]"),
                Some(&element_id),
                depth + 1,
                elements,
                references,
                complete,
            )?;
            if elements.len() >= 2000 {
                *complete = false;
                break;
            }
        }
    }
    Ok(())
}
