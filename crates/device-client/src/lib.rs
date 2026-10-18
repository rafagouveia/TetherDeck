//! Shared async client for native UI, CLI and automation clients.
use rust_ios_device_api::{
    API_VERSION, ApiProblem, DiagnosticReport, HidAction, RecoveryReport, TouchRequest,
    TouchResponse,
};
use serde::de::DeserializeOwned;
use std::time::Duration;
mod video;
pub use video::{VideoPacket, VideoStream};

#[derive(Debug, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ClientError {
    pub code: String,
    pub message: String,
    pub problem: Option<ApiProblem>,
}

fn error(code: &str, message: &str) -> ClientError {
    ClientError {
        code: code.into(),
        message: message.into(),
        problem: None,
    }
}

pub fn validate_origin(input: &str) -> Result<url::Url, ClientError> {
    let url = url::Url::parse(input)
        .map_err(|_| error("invalid_origin", "Informe uma origem HTTP de loopback."))?;
    let loopback = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    if url.scheme() != "http"
        || !loopback
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(error(
            "invalid_origin",
            "Use HTTP em IP de loopback, diretamente ou através de SSH.",
        ));
    }
    Ok(url)
}

#[derive(Clone)]
pub struct DeviceClient {
    http: reqwest::Client,
    base: String,
    token: String,
}

/// Bounded multipart receiver. No automatic reconnect or command replay.
pub struct ScreenStream {
    response: reqwest::Response,
    buffer: Vec<u8>,
}
impl ScreenStream {
    pub async fn next_frame(&mut self) -> Result<Vec<u8>, ClientError> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        loop {
            if self.buffer.starts_with(b"\r\n") {
                self.buffer.drain(..2);
            }
            if let Some(end) = self.buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                if end > 8192 || !self.buffer.starts_with(b"--tetherdeck-frame\r\n") {
                    return Err(error("invalid_stream", "Cabeçalho de vídeo inválido."));
                }
                let header = std::str::from_utf8(&self.buffer[..end])
                    .map_err(|_| error("invalid_stream", "Cabeçalho inválido."))?;
                let length = header
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .and_then(|s| s.parse::<usize>().ok())
                    .filter(|n| *n >= 4 && *n <= 16 * 1024 * 1024)
                    .ok_or_else(|| error("invalid_frame", "Tamanho de quadro inválido."))?;
                let start = end + 4;
                if self.buffer.len() >= start + length + 2 {
                    let frame = self.buffer[start..start + length].to_vec();
                    self.buffer.drain(..start + length + 2);
                    if !frame.starts_with(&[0xff, 0xd8]) {
                        return Err(error("invalid_frame", "Quadro JPEG inválido."));
                    }
                    return Ok(frame);
                }
            } else if self.buffer.len() > 8192 {
                return Err(error(
                    "invalid_stream",
                    "Cabeçalho de vídeo excedeu o limite.",
                ));
            }
            let chunk = tokio::time::timeout_at(deadline, self.response.chunk())
                .await
                .map_err(|_| {
                    error(
                        "stream_stalled",
                        "A tela parou de atualizar. Reconecte antes de controlar.",
                    )
                })?
                .map_err(|_| error("stream_lost", "A conexão da tela foi interrompida."))?
                .ok_or_else(|| error("stream_closed", "A transmissão foi encerrada."))?;
            if self.buffer.len() + chunk.len() > 16 * 1024 * 1024 + 16384 {
                return Err(error("invalid_stream", "Buffer de vídeo excedeu o limite."));
            }
            self.buffer.extend_from_slice(&chunk);
        }
    }
}

impl DeviceClient {
    pub async fn screen_stream(&self, id: &str) -> Result<ScreenStream, ClientError> {
        validate_device_id(id)?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .build()
            .map_err(|_| error("client_failed", "Não foi possível abrir a tela."))?;
        let response = tokio::time::timeout(
            Duration::from_secs(30),
            http.get(format!("{}/v1/devices/{id}/screen.mjpeg", self.base))
                .bearer_auth(&self.token)
                .header("x-protocol-version", API_VERSION.to_string())
                .send(),
        )
        .await
        .map_err(|_| {
            error(
                "stream_timeout",
                "O aparelho não iniciou a transmissão no prazo.",
            )
        })?
        .map_err(|_| {
            error(
                "stream_unreachable",
                "Não foi possível abrir a transmissão.",
            )
        })?;
        let mut response = response;
        if !response.status().is_success() {
            let status = response.status();
            let mut bytes = Vec::new();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            while let Ok(Ok(Some(chunk))) =
                tokio::time::timeout_at(deadline, response.chunk()).await
            {
                if bytes.len() + chunk.len() > 16384 {
                    break;
                }
                bytes.extend_from_slice(&chunk);
            }
            if let Ok(problem) = serde_json::from_slice::<ApiProblem>(&bytes) {
                return Err(ClientError {
                    code: problem.code.clone(),
                    message: problem.message.clone(),
                    problem: Some(problem),
                });
            }
            return Err(error(
                "stream_unavailable",
                &format!(
                    "Transmissão indisponível (HTTP {}). Consulte Diagnóstico.",
                    status.as_u16()
                ),
            ));
        }
        if response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            != Some("multipart/x-mixed-replace; boundary=tetherdeck-frame")
        {
            return Err(error(
                "invalid_stream",
                "O daemon não ofereceu um fluxo de tela compatível.",
            ));
        }
        Ok(ScreenStream {
            response,
            buffer: Vec::new(),
        })
    }
    pub fn new(origin: &str, token: &str) -> Result<Self, ClientError> {
        if token.trim().is_empty() {
            return Err(error("token_required", "Informe o token do daemon."));
        }
        let base = validate_origin(origin)?
            .as_str()
            .trim_end_matches('/')
            .to_owned();
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(110))
            .connect_timeout(Duration::from_secs(3))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| error("client_failed", "Não foi possível criar o cliente HTTP."))?;
        Ok(Self {
            http,
            base,
            token: token.into(),
        })
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<Vec<u8>, ClientError> {
        if !path.starts_with("/v1/") {
            return Err(error("invalid_path", "Use uma rota relativa da API /v1/."));
        }
        let mutation = method != reqwest::Method::GET && !path.ends_with("/ui/find");
        let mut request = self
            .http
            .request(method.clone(), format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .header("x-protocol-version", API_VERSION.to_string());
        if let Some(body) = body {
            request = request.json(body);
        }
        if path == "/v1/recovery/elements" {
            request = request.timeout(Duration::from_secs(150));
        } else if path.ends_with("/touch") {
            request = request.timeout(Duration::from_secs(1));
        }
        // No mutation is automatically replayed after a timeout or lost response.
        let mut response = request.send().await.map_err(|failure| {
            if failure.is_connect() { error("daemon_unreachable", "Não foi possível conectar ao daemon. No host, execute rust-ios-device-daemon --doctor; para acesso remoto, confira o túnel SSH.") }
            else if mutation { error("action_outcome_unknown", "A resposta da ação foi perdida; verifique o estado do aparelho antes de repetir.") }
            else { error("request_failed", "A leitura não completou; execute o diagnóstico do host.") }
        })?;
        let status = response.status();
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| {
            if mutation {
                error(
                    "action_outcome_unknown",
                    "A resposta foi interrompida; observe o aparelho antes de repetir a ação.",
                )
            } else {
                error(
                    "response_lost",
                    "A leitura foi interrompida. Obtenha uma observação atual.",
                )
            }
        })? {
            if bytes.len() + chunk.len() > 32 * 1024 * 1024 {
                return Err(error("response_too_large", "Resposta excedeu 32 MiB."));
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            if let Ok(problem) = serde_json::from_slice::<ApiProblem>(&bytes) {
                return Err(ClientError {
                    code: problem.code.clone(),
                    message: problem.message.clone(),
                    problem: Some(problem),
                });
            }
            return Err(error(
                "daemon_error",
                &format!("Daemon retornou HTTP {}.", status.as_u16()),
            ));
        }
        Ok(bytes)
    }

    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        serde_json::from_slice(&self.request(reqwest::Method::GET, path, None).await?).map_err(
            |_| {
                error(
                    "invalid_response",
                    "Resposta JSON incompatível com o cliente.",
                )
            },
        )
    }
    pub async fn json_request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value, ClientError> {
        serde_json::from_slice(&self.request(method, path, body).await?).map_err(|_| {
            error(
                "invalid_response",
                "Resposta JSON incompatível com o cliente.",
            )
        })
    }
    pub async fn health(&self) -> Result<serde_json::Value, ClientError> {
        let health: serde_json::Value = self.get_json("/v1/health").await?;
        if health["api_version"].as_u64() != Some(API_VERSION.into()) {
            return Err(error(
                "protocol_mismatch",
                "Atualize cliente e daemon para versões compatíveis.",
            ));
        }
        Ok(health)
    }
    pub async fn diagnostics(&self) -> Result<DiagnosticReport, ClientError> {
        self.get_json("/v1/diagnostics").await
    }
    pub async fn capabilities(&self) -> Result<serde_json::Value, ClientError> {
        self.get_json("/v1/capabilities").await
    }
    pub async fn devices(&self) -> Result<serde_json::Value, ClientError> {
        self.get_json("/v1/devices").await
    }
    pub async fn ui_snapshot(
        &self,
        id: &str,
    ) -> Result<rust_ios_device_api::UiSnapshot, ClientError> {
        self.ui_snapshot_with_provider(id, rust_ios_device_api::ElementProvider::Auto)
            .await
    }
    pub async fn ui_snapshot_with_provider(
        &self,
        id: &str,
        provider: rust_ios_device_api::ElementProvider,
    ) -> Result<rust_ios_device_api::UiSnapshot, ClientError> {
        validate_device_id(id)?;
        self.get_json(&format!(
            "/v1/devices/{id}/ui?provider={}",
            provider.as_str()
        ))
        .await
    }
    pub async fn find_element(
        &self,
        id: &str,
        request: &rust_ios_device_api::UiFindRequest,
    ) -> Result<rust_ios_device_api::UiMatch, ClientError> {
        validate_device_id(id)?;
        let body = serde_json::to_value(request)
            .map_err(|_| error("invalid_selector", "Seletor inválido."))?;
        let bytes = self
            .request(
                reqwest::Method::POST,
                &format!("/v1/devices/{id}/ui/find"),
                Some(&body),
            )
            .await?;
        serde_json::from_slice(&bytes)
            .map_err(|_| error("invalid_response", "Resposta de elemento inválida."))
    }
    pub async fn activate_element(
        &self,
        id: &str,
        request: &rust_ios_device_api::UiActivateRequest,
    ) -> Result<serde_json::Value, ClientError> {
        validate_device_id(id)?;
        let body = serde_json::to_value(request)
            .map_err(|_| error("invalid_reference", "Referência inválida."))?;
        let bytes = self
            .request(
                reqwest::Method::POST,
                &format!("/v1/devices/{id}/ui/activate"),
                Some(&body),
            )
            .await?;
        serde_json::from_slice(&bytes).map_err(|_| {
            error(
                "invalid_response",
                "Resposta de ativação inválida; não repetir automaticamente.",
            )
        })
    }
    pub async fn recover_service(&self) -> Result<RecoveryReport, ClientError> {
        serde_json::from_slice(
            &self
                .request(reqwest::Method::POST, "/v1/recovery/service", None)
                .await?,
        )
        .map_err(|_| error("invalid_response", "Resposta de recuperação inválida."))
    }
    pub async fn recover_elements(&self) -> Result<serde_json::Value, ClientError> {
        let bytes = self
            .request(reqwest::Method::POST, "/v1/recovery/elements", None)
            .await?;
        serde_json::from_slice(&bytes).map_err(|_| {
            error(
                "invalid_response",
                "Resposta da recuperação WDA inválida. Consulte o diagnóstico.",
            )
        })
    }
    pub async fn recover_usb_interface(
        &self,
        request: &rust_ios_device_api::UsbRecoveryRequest,
    ) -> Result<rust_ios_device_api::UsbRecoveryReport, ClientError> {
        validate_device_id(&request.device_id)?;
        let body = serde_json::to_value(request)
            .map_err(|_| error("invalid_usb_instance", "Instância USB inválida."))?;
        let bytes = self
            .request(
                reqwest::Method::POST,
                "/v1/recovery/usb-interface",
                Some(&body),
            )
            .await?;
        serde_json::from_slice(&bytes).map_err(|_| {
            error(
                "invalid_response",
                "Resposta de recuperação USB inválida; consulte diagnóstico antes de repetir.",
            )
        })
    }
    pub async fn screenshot(&self, id: &str) -> Result<Vec<u8>, ClientError> {
        validate_device_id(id)?;
        let bytes = self
            .request(
                reqwest::Method::GET,
                &format!("/v1/devices/{id}/screenshot.png"),
                None,
            )
            .await?;
        if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err(error("invalid_png", "O daemon não retornou um PNG válido."));
        }
        Ok(bytes)
    }
    /// One live touch phase, never retried after timeout or a missing response.
    pub async fn touch(
        &self,
        id: &str,
        touch: &TouchRequest,
    ) -> Result<TouchResponse, ClientError> {
        validate_device_id(id)?;
        let body = serde_json::to_value(touch)
            .map_err(|_| error("invalid_touch", "Fase de toque inválida."))?;
        let bytes = self
            .request(
                reqwest::Method::POST,
                &format!("/v1/devices/{id}/touch"),
                Some(&body),
            )
            .await?;
        let response: TouchResponse = serde_json::from_slice(&bytes).map_err(|_| {
            error(
                "action_outcome_unknown",
                "A resposta do toque é incompatível; não repetir a fase automaticamente.",
            )
        })?;
        if response.gesture_id != touch.gesture_id || response.sequence != touch.sequence {
            return Err(error(
                "action_outcome_unknown",
                "A resposta não corresponde à fase enviada; não repetir automaticamente.",
            ));
        }
        Ok(response)
    }
    pub async fn action(&self, id: &str, action: &HidAction) -> Result<(), ClientError> {
        validate_device_id(id)?;
        let body =
            serde_json::to_value(action).map_err(|_| error("invalid_action", "Ação inválida."))?;
        self.request(
            reqwest::Method::POST,
            &format!("/v1/devices/{id}/actions"),
            Some(&body),
        )
        .await?;
        Ok(())
    }
    pub async fn launch(&self, id: &str, bundle: &str) -> Result<serde_json::Value, ClientError> {
        validate_device_id(id)?;
        let bytes = self
            .request(
                reqwest::Method::POST,
                &format!("/v1/devices/{id}/apps"),
                Some(&serde_json::json!({"bundle_id":bundle})),
            )
            .await?;
        serde_json::from_slice(&bytes)
            .map_err(|_| error("invalid_response", "Resposta de abertura inválida."))
    }
}

/// Validate a device identifier before using it in a route or local daemon command.
pub fn validate_device_id(id: &str) -> Result<(), ClientError> {
    if id.is_empty()
        || id.len() > 128
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(error(
            "invalid_device_id",
            "Use o device_id retornado pela listagem.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn origin_does_not_allow_token_exfiltration_or_ambiguous_routes() {
        for bad in [
            "http://127.0.0.1@evil.example",
            "http://localhost:123",
            "http://192.168.0.2",
            "http://127.0.0.1/api",
            "http://127.0.0.1?x=1",
        ] {
            assert!(validate_origin(bad).is_err());
        }
        assert!(validate_origin("http://[::1]:18087").is_ok());
    }
}
