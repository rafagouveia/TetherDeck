//! A persistent capture service: one authenticated connection, consecutive full-size frames.
use crate::*;
type Capture = ScreenCaptureServiceClient<Box<dyn ReadWrite>>;
pub struct Screen {
    capture: Capture,
    // Keep the software tunnel alive for USB or direct paired Wi-Fi.
    _tunnel: Option<idevice::tcp::handle::AdapterHandle>,
}
impl Screen {
    pub async fn connect(
        id: String,
        rsd: Option<(IpAddr, u16, String)>,
    ) -> Result<Self, DeviceError> {
        tokio::time::timeout(Duration::from_secs(20), async {
            if let Some((host, port, expected)) = rsd {
                let stream = tokio::net::TcpStream::connect((host, port)).await?;
                let mut handshake = RsdHandshake::new(stream).await?;
                if handshake.uuid != expected {
                    return Err(DeviceError::DeviceIdMismatch);
                }
                let mut provider = host;
                let capture = handshake.connect(&mut provider).await?;
                Ok(Self {
                    capture,
                    _tunnel: None,
                })
            } else {
                let (mut tunnel, mut handshake) = super::usb_rsd_stack(id).await?;
                let capture = handshake.connect(&mut tunnel).await?;
                Ok(Self {
                    capture,
                    _tunnel: Some(tunnel),
                })
            }
        })
        .await
        .map_err(|_| DeviceError::Timeout)?
    }
    pub async fn frame(&mut self) -> Result<Vec<u8>, DeviceError> {
        let bytes = tokio::time::timeout(
            Duration::from_secs(5),
            self.capture.take_screenshot(None, ImageFormat::Png),
        )
        .await
        .map_err(|_| DeviceError::Timeout)??;
        if bytes.len() > 16 * 1024 * 1024 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err(DeviceError::LiveFrameUnavailable);
        }
        // The device's capture service supports PNG reliably. Encode JPEG on the host;
        // no per-frame connection setup and no scaling of coordinates.
        let mut reader =
            image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(4096);
        limits.max_alloc = Some(128 * 1024 * 1024);
        reader.limits(limits);
        let image = reader
            .decode()
            .map_err(|_| DeviceError::LiveFrameUnavailable)?
            .to_rgb8();
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 65)
            .encode_image(&image)
            .map_err(|_| DeviceError::LiveFrameUnavailable)?;

        Ok(bytes)
    }
}
