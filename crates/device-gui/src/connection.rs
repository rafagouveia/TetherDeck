//! A GUI-owned daemon scoped to one phone, with credentials only in process memory.
use super::*;
use std::{path::PathBuf, process::Stdio};

pub(super) struct LocalConnection {
    pub mode: u8,
    pub runner_configured: bool,
    _child: tokio::process::Child,
    pub origin: String,
    pub token: String,
    pub log: PathBuf,
}
fn sibling(name: &str) -> Result<PathBuf, String> {
    let mut path = std::env::current_exe()
        .map_err(|_| "Não foi possível localizar a pasta do programa.".to_owned())?;
    path.set_file_name(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    });
    Ok(path)
}
async fn start(
    id: String,
    bundle: String,
    allow_usb: bool,
    mode: u8,
    wifi_host: Option<String>,
    tunneld: String,
) -> Result<(LocalConnection, DiagnosticReport), String> {
    if id.is_empty()
        || id.len() > 128
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || bundle.len() > 255
        || !bundle.contains('.')
        || !bundle
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    {
        return Err("Identificador de aparelho ou runner inválido.".into());
    }
    let socket = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .map_err(|_| "Não foi possível reservar uma porta local.".to_owned())?;
    let address = socket
        .local_addr()
        .map_err(|_| "Não foi possível consultar a porta local.".to_owned())?;
    drop(socket);
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let origin = format!("http://{address}");
    let log_dir = std::env::temp_dir().join(format!("rust-ios-device-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&log_dir).map_err(|_| "Não foi possível criar o log local.".to_owned())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&log_dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "Não foi possível restringir o log local.".to_owned())?;
    }
    let log = log_dir.join("daemon.log");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&log)
        .map_err(|_| "Não foi possível abrir o log local.".to_owned())?;
    let error_log = file
        .try_clone()
        .map_err(|_| "Não foi possível preparar o log local.".to_owned())?;
    let mut command = tokio::process::Command::new(sibling("rust-ios-device-daemon")?);
    command
        .args(["--listen", &address.to_string(), "--device", &id])
        .env("RUST_IOS_DEVICE_TOKEN", &token)
        .env("RUST_LOG", "info")
        .stdin(Stdio::null())
        .stdout(Stdio::from(file))
        .stderr(Stdio::from(error_log))
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    if allow_usb && cfg!(windows) {
        command.arg("--allow-usb-interface-restart");
    }
    if mode == 2 {
        command.args(["--tunneld-url", &tunneld]);
    } else if mode == 1 {
        command.args(["--transport", "wifi", "--wifi-direct"]);
        if let Some(host) = &wifi_host {
            command.args(["--wifi-host", host]);
        }
    } else {
        command.args([
            "--transport",
            if mode == 1 { "wifi" } else { "usb" },
            "--wda-runner",
            &bundle,
            "--require-wda",
        ]);
    }
    let mut child = command.spawn().map_err(|_| {
        "Não foi possível iniciar o daemon. Mantenha os executáveis na mesma pasta.".to_owned()
    })?;
    let client = DeviceClient::new(&origin, &token).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(150);
    loop {
        if child
            .try_wait()
            .map_err(|_| "Não foi possível consultar o processo local.".to_owned())?
            .is_some()
        {
            return Err(format!(
                "O daemon local encerrou. Consulte {}.",
                log.display()
            ));
        }
        if matches!(
            tokio::time::timeout(Duration::from_secs(1), client.health()).await,
            Ok(Ok(_))
        ) {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "A conexão local excedeu o prazo. Consulte {}.",
                log.display()
            ));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let report = tokio::time::timeout(Duration::from_secs(25), client.diagnostics())
        .await
        .map_err(|_| "O diagnóstico da nova conexão excedeu o prazo.".to_owned())?
        .map_err(|e| e.to_string())?;
    Ok((
        LocalConnection {
            mode,
            runner_configured: mode == 0,
            _child: child,
            origin,
            token,
            log,
        },
        report,
    ))
}
impl Studio {
    pub(super) fn connect_local(&mut self, id: String, ctx: &egui::Context) {
        if self.busy || self.local_connection.is_some() {
            return;
        }
        self.busy = true;
        self.error = None;
        self.elements = None;
        self.texture = None;
        self.snapshot_at = None;
        self.control_enabled = false;
        let bundle = self.runner_bundle.clone();
        let allow_usb = self.local_allow_usb_recovery;
        let mode = self.connection_mode;
        let wifi_host = self
            .report
            .as_ref()
            .and_then(|r| {
                r.devices
                    .iter()
                    .find(|d| d.udid.as_deref() == Some(id.as_str()))
            })
            .and_then(|d| d.transport.strip_prefix("network:"))
            .map(str::to_owned);
        let tunneld = self.tunneld_origin.clone();
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        let context = ctx.clone();
        self.runtime.spawn(async move {
            let event = match start(id, bundle, allow_usb, mode, wifi_host, tunneld).await {
                Ok((connection, report)) => Event::Connected(connection, report),
                Err(error) => Event::Failed(error),
            };
            let _ = sender.send(event);
            context.request_repaint();
        });
    }
    pub(super) fn copy_mcp_config(&self, ctx: &egui::Context) {
        let Some(connection) = &self.local_connection else {
            return;
        };
        let Ok(command) = sibling("rust-ios-device-mcp") else {
            return;
        };
        ctx.copy_text(serde_json::json!({"mcpServers":{"rust-ios-device":{"command":command,"env":{"RUST_IOS_DEVICE_URL":connection.origin,"RUST_IOS_DEVICE_TOKEN":connection.token}}}}).to_string());
    }
}

/// A single existing remote pairing record can suggest a UDID without reading keys.
pub(super) fn known_wifi_device() -> String {
    let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) else {
        return String::new();
    };
    let Ok(entries) = std::fs::read_dir(std::path::PathBuf::from(home).join(".pymobiledevice3"))
    else {
        return String::new();
    };
    let ids: Vec<_> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            let id = name.strip_prefix("remote_")?.strip_suffix(".plist")?;
            let normalized = id.replace('-', "");
            (matches!(normalized.len(), 24 | 40)
                && normalized.bytes().all(|c| c.is_ascii_hexdigit()))
            .then(|| id.to_owned())
        })
        .collect();
    if ids.len() == 1 {
        ids[0].clone()
    } else {
        String::new()
    }
}
