use super::*;
use std::time::Instant;

/// An encoded access unit with its original daemon metadata. elapsed_us is the
/// daemon's monotonic elapsed time at AU emission, not an iPhone capture time.
#[derive(Debug)]
pub struct VideoPacket {
    pub bytes: Vec<u8>,
    pub rtp_timestamp: u32,
    pub sequence: u32,
    pub keyframe: bool,
    pub elapsed_us: u64,
    pub received_at: Instant,
}

/// Complete encoded access units; HTTP chunk boundaries carry no meaning.
pub struct VideoStream {
    response: reqwest::Response,
    buffer: Vec<u8>,
    previous_sequence: Option<u32>,
    previous_elapsed_us: Option<u64>,
    pub source_size: [usize; 2],
    /// Original decoder-visible SPS dimensions, before host scaling. These use
    /// the display headers so codec conformance cropping is already accounted for.
    /// Unknown metadata remains [0, 0]; never infer physical HID geometry from PAM.
    pub coded_size: [usize; 2],
}
impl VideoStream {
    pub async fn next_frame(&mut self) -> Result<Vec<u8>, ClientError> {
        self.next_packet().await.map(|packet| packet.bytes)
    }

    pub async fn next_packet(&mut self) -> Result<VideoPacket, ClientError> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        loop {
            if self.buffer.len() >= 24 {
                let length = u32::from_le_bytes(self.buffer[..4].try_into().unwrap()) as usize;
                if !(4..=8 * 1024 * 1024).contains(&length) {
                    return Err(error("invalid_video", "Unidade HEVC inválida."));
                }
                if self.buffer.len() >= 24 + length {
                    let rtp_timestamp = u32::from_le_bytes(self.buffer[4..8].try_into().unwrap());
                    let sequence = u32::from_le_bytes(self.buffer[8..12].try_into().unwrap());
                    let flags = u32::from_le_bytes(self.buffer[12..16].try_into().unwrap());
                    let elapsed_us = u64::from_le_bytes(self.buffer[16..24].try_into().unwrap());
                    if self.previous_sequence.is_some_and(|previous| {
                        let advance = sequence.wrapping_sub(previous);
                        advance == 0 || advance >= 0x8000_0000
                    }) || self
                        .previous_elapsed_us
                        .is_some_and(|previous| elapsed_us < previous)
                    {
                        return Err(error(
                            "invalid_video_timing",
                            "A sequência ou o relógio do vídeo regrediu. Retome a transmissão.",
                        ));
                    }
                    let packet = VideoPacket {
                        bytes: self.buffer[24..24 + length].to_vec(),
                        rtp_timestamp,
                        sequence,
                        keyframe: flags & 1 != 0,
                        elapsed_us,
                        received_at: Instant::now(),
                    };
                    self.buffer.drain(..24 + length);
                    self.previous_sequence = Some(sequence);
                    self.previous_elapsed_us = Some(elapsed_us);
                    return Ok(packet);
                }
            }
            let chunk = tokio::time::timeout_at(deadline, self.response.chunk())
                .await
                .map_err(|_| {
                    error(
                        "video_stalled",
                        "O vídeo parou de atualizar. O fluxo atual foi encerrado.",
                    )
                })?
                .map_err(|_| error("video_lost", "A conexão de vídeo foi interrompida."))?
                .ok_or_else(|| error("video_closed", "O serviço encerrou o vídeo."))?;
            if self.buffer.len() + chunk.len() > 8 * 1024 * 1024 + 65536 {
                return Err(error("invalid_video", "Buffer HEVC excedeu o limite."));
            }
            self.buffer.extend_from_slice(&chunk);
        }
    }
}
impl DeviceClient {
    pub async fn video_stream(&self, id: &str) -> Result<VideoStream, ClientError> {
        validate_device_id(id)?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .build()
            .map_err(|_| error("client_failed", "Não foi possível abrir o vídeo."))?;
        let mut response = tokio::time::timeout(
            Duration::from_secs(45),
            http.get(format!("{}/v1/devices/{id}/screen.video", self.base))
                .bearer_auth(&self.token)
                .header("x-protocol-version", API_VERSION.to_string())
                .send(),
        )
        .await
        .map_err(|_| error("video_timeout", "O iPhone não iniciou o vídeo no prazo."))?
        .map_err(|_| error("video_unreachable", "Não foi possível abrir o vídeo."))?;
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
                match status {
                    reqwest::StatusCode::LOCKED => "video_busy",
                    reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
                        "video_unauthorized"
                    }
                    reqwest::StatusCode::BAD_REQUEST => "invalid_video_request",
                    reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED => {
                        "video_unsupported"
                    }
                    _ => "video_unavailable",
                },
                &format!(
                    "Vídeo indisponível (HTTP {}). Consulte Diagnóstico.",
                    status.as_u16()
                ),
            ));
        }
        if response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            != Some("application/vnd.tetherdeck.hevc")
            || response
                .headers()
                .get("x-tetherdeck-video")
                .and_then(|v| v.to_str().ok())
                != Some("1")
        {
            return Err(error("invalid_video", "Versão de vídeo incompatível."));
        }
        let size = |name| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<usize>().ok())
                .filter(|v| *v <= 4096)
                .unwrap_or(0)
        };
        let source_size = [size("x-source-width"), size("x-source-height")];
        let coded_size = [size("x-hevc-display-width"), size("x-hevc-display-height")];
        Ok(VideoStream {
            response,
            buffer: Vec::new(),
            previous_sequence: None,
            previous_elapsed_us: None,
            source_size,
            coded_size,
        })
    }
}
