//! Native encrypted RemotePairing tunnel using an existing local pairing record.
//! No driver, privileged TUN interface or Python runtime is needed.
use crate::*;
use idevice::remote_pairing::{
    RemotePairingClient, RpPairingFile, RpPairingSocket, connect_tls_psk_tunnel_native,
    tls_psk::TlsPskStream,
};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
type Control = RemotePairingClient<RpPairingSocket<tokio::net::TcpStream>>;
struct KeepControl {
    stream: TlsPskStream<tokio::net::TcpStream>,
    _control: Control,
}
impl std::fmt::Debug for KeepControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PairedWifiTunnel")
    }
}
impl AsyncRead for KeepControl {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}
impl AsyncWrite for KeepControl {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
pub(crate) async fn connect(
    id: &str,
) -> Result<(idevice::tcp::handle::AdapterHandle, RsdHandshake), DeviceError> {
    // First authenticate the selected phone via pinned Lockdown TLS.
    let summary = crate::wireless::summary().await?;
    if summary.udid.as_deref() != Some(id) {
        return Err(DeviceError::DeviceIdMismatch);
    }
    let host: IpAddr = summary
        .transport
        .strip_prefix("network:")
        .ok_or(DeviceError::InvalidRsdAddress)?
        .parse()
        .map_err(|_| DeviceError::InvalidRsdAddress)?;
    let hostname = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "localhost".into());
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .ok_or(DeviceError::MissingRsdPort)?;
    let path = std::path::PathBuf::from(home)
        .join(".pymobiledevice3")
        .join(format!("remote_{id}.plist"));
    if tokio::fs::metadata(&path)
        .await
        .map_err(|_| DeviceError::WifiPairingRequired)?
        .len()
        > 16384
    {
        return Err(DeviceError::MissingRsdPort);
    }
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|_| DeviceError::WifiPairingRequired)?;
    let mut record: plist::Dictionary =
        plist::from_bytes(&bytes).map_err(idevice::IdeviceError::from)?;
    // Import the interoperable existing record without logging or copying its secret material.
    if !record.contains_key("identifier") {
        record.insert(
            "identifier".into(),
            RpPairingFile::generate(&hostname).identifier.into(),
        );
    }
    let record_bytes = plist::to_value(&record)
        .and_then(|v| {
            let mut bytes = Vec::new();
            v.to_writer_xml(&mut bytes)?;
            Ok(bytes)
        })
        .map_err(idevice::IdeviceError::from)?;
    let mut pair = RpPairingFile::from_bytes(&record_bytes)?;
    let daemon = mdns_sd::ServiceDaemon::new().map_err(|_| DeviceError::MissingRsdPort)?;
    struct Stop(mdns_sd::ServiceDaemon);
    impl Drop for Stop {
        fn drop(&mut self) {
            let _ = self.0.shutdown();
        }
    }
    let _stop = Stop(daemon.clone());
    let receiver = daemon
        .browse("_remotepairing._tcp.local.")
        .map_err(|_| DeviceError::MissingRsdPort)?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut port = None;
    while let Ok(Ok(event)) = tokio::time::timeout_at(deadline, receiver.recv_async()).await {
        if let mdns_sd::ServiceEvent::ServiceResolved(service) = event {
            if service.port != 0 && service.addresses.iter().any(|ip| ip.to_ip_addr() == host) {
                port = Some(service.port);
                break;
            }
        }
    }
    let socket =
        tokio::net::TcpStream::connect((host, port.ok_or(DeviceError::MissingRsdPort)?)).await?;
    socket.set_nodelay(true)?;
    let mut control = RemotePairingClient::new(RpPairingSocket::new(socket), &hostname);
    control.attempt_pair_verify().await?;
    // Verification only: never silently initiate a new pairing or replace the user's record.
    control.validate_pairing(&mut pair).await?;
    let port = control.create_tcp_listener().await?;
    let stream = tokio::net::TcpStream::connect((host, port)).await?;
    // Media ACKs and input share the tunnel: do not hold small feedback packets
    // waiting for another application write or a delayed TCP acknowledgement.
    stream.set_nodelay(true)?;
    let tunnel = Box::pin(connect_tls_psk_tunnel_native(
        stream,
        control.encryption_key(),
    ))
    .await?;
    let rsd_port = tunnel.info.server_rsd_port;
    let ours: IpAddr = tunnel
        .info
        .client_address
        .parse()
        .map_err(|_| DeviceError::InvalidRsdAddress)?;
    let theirs: IpAddr = tunnel
        .info
        .server_address
        .parse()
        .map_err(|_| DeviceError::InvalidRsdAddress)?;
    let mtu = tunnel.info.mtu as usize;
    let stream = KeepControl {
        stream: tunnel.into_inner(),
        _control: control,
    };
    let mut adapter = idevice::tcp::adapter::Adapter::new(Box::new(stream), ours, theirs);
    adapter.set_mss(mtu.saturating_sub(60));
    let mut handle = adapter.to_async_handle();
    let stream = handle.connect_to_service_port(rsd_port).await?;
    let handshake = RsdHandshake::new(stream).await?;
    let actual = handshake
        .properties
        .get("UniqueDeviceID")
        .and_then(|v| v.as_string())
        .unwrap_or("");
    if actual.replace('-', "") != id.replace('-', "") {
        return Err(DeviceError::DeviceIdMismatch);
    }
    Ok((handle, handshake))
}
