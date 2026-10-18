//! External Windows decoder. Encoded frames remain ordered; only decoded RGB is coalesced.
use rust_ios_device_client::DeviceClient;
use std::{
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

fn executable() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("TETHERDECK_FFMPEG") {
        let path = PathBuf::from(path);
        return path
            .is_file()
            .then_some(path)
            .ok_or("TETHERDECK_FFMPEG não aponta para um executável.".into());
    }
    let mut paths = Vec::new();
    if let Some(parent) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
    {
        paths.push(parent.join("ffmpeg.exe"));
    }
    if let Some(programs) = std::env::var_os("ProgramFiles") {
        paths.push(PathBuf::from(programs).join("ffmpeg/ffmpeg.exe"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path).map(|p| p.join("ffmpeg.exe")));
    }
    paths.into_iter().find(|p| p.is_file()).ok_or(
        "Instale FFmpeg com decoder HEVC ou configure TETHERDECK_FFMPEG para abrir a tela ao vivo."
            .into(),
    )
}

struct Writer(tokio::task::JoinHandle<Result<(), String>>);

const MAX_RECEIVE_DRIFT: Duration = Duration::from_secs(1);
const DRIFT_SUSTAIN: Duration = Duration::from_millis(750);
const PIPE_WRITE_BUDGET: Duration = Duration::from_millis(1500);
const DRIFT_STOP: &str = "A recepção acumulou mais de 1 s de atraso adicional de forma sustentada. A tela foi encerrada; retome a transmissão.";
const PIPE_STOP: &str = "O decoder bloqueou a entrada por mais de 1,5 s. A tela foi encerrada para evitar atraso acumulado; retome a transmissão.";

#[derive(Clone, Copy, Default)]
struct ReceiveTiming {
    received_units: u64,
    daemon_sequence: u32,
    daemon_elapsed_us: u64,
    local_since_first_us: u64,
    daemon_since_first_us: u64,
    relative_drift_us: i64,
    max_positive_drift_us: u64,
    pipe_write_us: u64,
    max_pipe_write_us: u64,
}
impl Drop for Writer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub async fn run(
    client: DeviceClient,
    target: String,
    mut frame: impl FnMut(eframe::egui::ColorImage, [usize; 2], u64) -> bool,
) -> Result<(), String> {
    let mode = std::env::var("TETHERDECK_VIDEO_DECODER").unwrap_or_else(|_| "auto".into());
    if !matches!(mode.as_str(), "auto" | "software" | "d3d11va") {
        return Err("TETHERDECK_VIDEO_DECODER aceita auto, software ou d3d11va.".into());
    }
    let mut opened = false;
    let mut produced = false;
    let hardware = cfg!(windows) && mode != "software";
    let result = run_mode(
        client.clone(),
        &target,
        hardware,
        &mut frame,
        &mut opened,
        &mut produced,
    )
    .await;
    let guard_stopped = result.as_ref().err().is_some_and(|message| {
        message == DRIFT_STOP
            || message == PIPE_STOP
            || message.starts_with("invalid_video_timing:")
    });
    if mode == "auto" && hardware && opened && !produced && !guard_stopped && result.is_err() {
        // Only retry a read-only viewer startup, before any frame was exposed.
        // GPU/driver incompatibility must not prevent software-only machines.
        run_mode(
            client,
            &target,
            false,
            &mut frame,
            &mut opened,
            &mut produced,
        )
        .await
    } else {
        result
    }
}

async fn run_mode(
    client: DeviceClient,
    target: &str,
    hardware: bool,
    frame: &mut impl FnMut(eframe::egui::ColorImage, [usize; 2], u64) -> bool,
    opened: &mut bool,
    produced: &mut bool,
) -> Result<(), String> {
    let mut command = tokio::process::Command::new(executable()?);
    if hardware {
        command.args(["-hwaccel", "d3d11va"]);
    }
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-flags",
            "low_delay",
            "-probesize",
            "32",
            "-analyzeduration",
            "0",
            "-threads",
            "2",
            "-f",
            "hevc",
            "-i",
            "pipe:0",
            "-an",
            "-sn",
            "-fps_mode",
            "passthrough",
            "-pix_fmt",
            "rgba",
            "-c:v",
            "pam",
            "-f",
            "image2pipe",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command
        .spawn()
        .map_err(|_| "Não foi possível iniciar o decoder FFmpeg.".to_owned())?;
    let mut input = child
        .stdin
        .take()
        .ok_or("Decoder sem entrada.".to_owned())?;
    let output = child.stdout.take().ok_or("Decoder sem saída.".to_owned())?;
    // A recently closed read-only session may still be stopping its media
    // service. Retry only that explicit busy result; never replay an input.
    let opening_deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut stream = loop {
        match client.video_stream(&target).await {
            Ok(stream) => break stream,
            Err(error)
                if error.code == "video_busy" && tokio::time::Instant::now() < opening_deadline =>
            {
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
            Err(error) => return Err(error.to_string()),
        }
    };
    let source_size = stream.source_size;
    let coded_size = stream.coded_size;
    *opened = true;
    let (timing_tx, timing_rx) = tokio::sync::watch::channel(ReceiveTiming::default());
    let mut writer = Writer(tokio::spawn(async move {
        let mut anchor: Option<(Instant, u64)> = None;
        let mut lag_started: Option<Instant> = None;
        let mut timing = ReceiveTiming::default();
        loop {
            let packet = stream.next_packet().await.map_err(|e| e.to_string())?;
            let (local_anchor, daemon_anchor) =
                *anchor.get_or_insert((packet.received_at, packet.elapsed_us));
            timing.received_units += 1;
            timing.daemon_sequence = packet.sequence;
            timing.daemon_elapsed_us = packet.elapsed_us;
            timing.local_since_first_us = packet
                .received_at
                .duration_since(local_anchor)
                .as_micros()
                .min(u64::MAX as u128) as u64;
            timing.daemon_since_first_us = packet
                .elapsed_us
                .checked_sub(daemon_anchor)
                .ok_or_else(|| "O relógio do daemon regrediu. Retome a transmissão.".to_owned())?;
            timing.relative_drift_us = (i128::from(timing.local_since_first_us)
                - i128::from(timing.daemon_since_first_us))
            .clamp(i128::from(i64::MIN), i128::from(i64::MAX))
                as i64;
            timing.max_positive_drift_us = timing
                .max_positive_drift_us
                .max(timing.relative_drift_us.max(0) as u64);
            timing_tx.send_replace(timing);

            // This detects added queueing after the first AU. It cannot measure
            // fixed startup delay, iPhone capture/encoding, or queues before the
            // daemon's emission timestamp. Cross-host clock-rate drift also counts.
            if timing.relative_drift_us > MAX_RECEIVE_DRIFT.as_micros() as i64 {
                let began = *lag_started.get_or_insert(packet.received_at);
                if packet.received_at.duration_since(began) >= DRIFT_SUSTAIN {
                    return Err(DRIFT_STOP.into());
                }
            } else {
                lag_started = None;
            }

            let writing = Instant::now();
            let result = tokio::time::timeout(PIPE_WRITE_BUDGET, async {
                input.write_all(&packet.bytes).await?;
                input.flush().await
            })
            .await;
            timing.pipe_write_us = writing.elapsed().as_micros().min(u64::MAX as u128) as u64;
            timing.max_pipe_write_us = timing.max_pipe_write_us.max(timing.pipe_write_us);
            timing_tx.send_replace(timing);
            result
                .map_err(|_| PIPE_STOP.to_owned())?
                .map_err(|_| "O decoder de vídeo encerrou a entrada.".to_owned())?;
        }
    }));
    let mut output = BufReader::with_capacity(256 * 1024, output);
    let mut sequence = 0;
    let started = std::time::Instant::now();
    let mut last_image_size = [0, 0];
    loop {
        let read = tokio::time::timeout(Duration::from_secs(10), read_frame(&mut output));
        let image = tokio::select! {
            result = &mut writer.0 => {
                let timing = *timing_rx.borrow();
                record_metrics(hardware, sequence, started, last_image_size, source_size, coded_size, timing, true).await;
                return result.unwrap_or_else(|_| Err("Recepção HEVC interrompida.".into()));
            },
            result = read => result.map_err(|_| "O decoder não produziu imagens no prazo. Consulte o diagnóstico de vídeo.".to_owned())??,
        };
        sequence += 1;
        last_image_size = image.size;
        *produced = true;
        if sequence % 300 == 0 {
            let timing = *timing_rx.borrow();
            record_metrics(
                hardware,
                sequence,
                started,
                image.size,
                source_size,
                coded_size,
                timing,
                false,
            )
            .await;
        }
        if !frame(image, source_size, sequence) {
            return Ok(());
        }
    }
}

async fn record_metrics(
    hardware: bool,
    decoded_frames: u64,
    started: Instant,
    image: [usize; 2],
    source: [usize; 2],
    coded: [usize; 2],
    timing: ReceiveTiming,
    ended: bool,
) {
    let Some(path) = std::env::var_os("TETHERDECK_VIDEO_METRICS") else {
        return;
    };
    let value = serde_json::json!({
        "decoder": if hardware { "d3d11va" } else { "software" },
        "decoded_frames": decoded_frames, "elapsed_seconds": started.elapsed().as_secs_f64(),
        "width": image[0], "height": image[1], "source_width": source[0], "source_height": source[1],
        "original_video_width": coded[0], "original_video_height": coded[1],
        "preview_width_limit": 720,
        "ended": ended, "received_units": timing.received_units,
        "daemon_sequence": timing.daemon_sequence, "daemon_elapsed_us": timing.daemon_elapsed_us,
        "local_since_first_us": timing.local_since_first_us,
        "daemon_since_first_us": timing.daemon_since_first_us,
        "relative_receive_drift_us": timing.relative_drift_us,
        "max_positive_receive_drift_us": timing.max_positive_drift_us,
        "last_pipe_write_us": timing.pipe_write_us, "max_pipe_write_us": timing.max_pipe_write_us,
        "receive_drift_limit_us": MAX_RECEIVE_DRIFT.as_micros() as u64,
        "receive_drift_sustain_us": DRIFT_SUSTAIN.as_micros() as u64,
        "pipe_write_budget_us": PIPE_WRITE_BUDGET.as_micros() as u64,
        "latency_measurement": "relative_daemon_emission_to_client_receive_drift_only",
        "latency_limits": "Anchored to the first AU; excludes initial fixed buffering, iPhone capture/encoding, pre-emission queues and display presentation. Different-host clock-rate drift can affect this estimate. Decoded FPS does not establish absolute latency.",
    });
    let _ = tokio::fs::write(path, value.to_string()).await;
}

async fn read_frame(
    reader: &mut BufReader<tokio::process::ChildStdout>,
) -> Result<eframe::egui::ColorImage, String> {
    let mut header = String::new();
    if reader
        .read_line(&mut header)
        .await
        .map_err(|_| "Falha na saída do decoder.".to_owned())?
        == 0
    {
        return Err("O decoder encerrou o vídeo. Verifique o suporte HEVC do FFmpeg.".into());
    }
    if header.trim() != "P7" {
        return Err("Formato de imagem inesperado no decoder.".into());
    }
    let (mut width, mut height, mut depth, mut maxval) = (0usize, 0usize, 0usize, 0usize);
    let mut total = header.len();
    loop {
        header.clear();
        if reader
            .read_line(&mut header)
            .await
            .map_err(|_| "Cabeçalho do decoder incompleto.".to_owned())?
            == 0
        {
            return Err("Decoder encerrado.".into());
        }
        total += header.len();
        if total > 8192 {
            return Err("Cabeçalho de imagem grande demais.".into());
        }
        let line = header.trim();
        if line == "ENDHDR" {
            break;
        }
        let mut fields = line.split_whitespace();
        let name = fields.next().unwrap_or("");
        let value = fields.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        match name {
            "WIDTH" => width = value,
            "HEIGHT" => height = value,
            "DEPTH" => depth = value,
            "MAXVAL" => maxval = value,
            _ => {}
        }
    }
    if width == 0 || height == 0 || width > 4096 || height > 4096 || depth != 4 || maxval != 255 {
        return Err("Dimensões ou pixels de vídeo incompatíveis.".into());
    }
    let mut bytes = vec![0; width * height * 4];
    reader
        .read_exact(&mut bytes)
        .await
        .map_err(|_| "Quadro do decoder incompleto.".to_owned())?;
    Ok(eframe::egui::ColorImage::from_rgba_unmultiplied(
        [width, height],
        &bytes,
    ))
}
