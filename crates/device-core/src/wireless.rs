//! Direct paired Lockdown over the LAN. Bonjour is discovery, never proof of identity.
use crate::*;
use idevice::{
    pairing_file::PairingFile,
    provider::{IdeviceProvider, TcpProvider},
};
use std::{net::IpAddr, sync::OnceLock};

struct Direct {
    id: String,
    host: Option<IpAddr>,
    resolved: tokio::sync::Mutex<Option<IpAddr>>,
    pair: tokio::sync::OnceCell<PairingFile>,
}
static DIRECT: OnceLock<Direct> = OnceLock::new();
pub fn configure(id: String, host: Option<IpAddr>) -> Result<(), &'static str> {
    if id.is_empty()
        || id.len() > 128
        || !id.bytes().all(|c| c.is_ascii_hexdigit() || c == b'-')
        || host.is_some_and(|ip| !allowed(ip))
    {
        return Err("direct Wi-Fi requires a hardware UDID and a private LAN IP");
    }
    DIRECT
        .set(Direct {
            id,
            host,
            resolved: Default::default(),
            pair: Default::default(),
        })
        .map_err(|_| "Wi-Fi already configured")
}
pub fn id() -> Option<&'static str> {
    DIRECT.get().map(|d| d.id.as_str())
}
fn allowed(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_link_local(),
        IpAddr::V6(ip) => (ip.segments()[0] & 0xfe00) == 0xfc00,
    }
}
fn same(a: &str, b: &str) -> bool {
    a.replace('-', "").eq_ignore_ascii_case(&b.replace('-', ""))
}
async fn pair(d: &Direct) -> Result<PairingFile, DeviceError> {
    d.pair
        .get_or_try_init(|| async {
            let mut mux = UsbmuxdAddr::from_env_var()?.connect(0).await?;
            mux.get_pair_record(&d.id).await.map_err(DeviceError::from)
        })
        .await
        .cloned()
}
async fn verify(
    host: IpAddr,
    id: &str,
    pair: PairingFile,
) -> Result<(TcpProvider, DeviceSummary), DeviceError> {
    tokio::time::timeout(Duration::from_secs(4), async {
        let provider = TcpProvider {
            addr: host,
            scope_id: None,
            pairing_file: pair,
            label: "tetherdeck-wifi".into(),
        };
        let mut lockdown = LockdownClient::connect(&provider).await?;
        // Our idevice patch pins TLS to the device certificate in the trusted pairing record.
        lockdown.start_session(&provider.pairing_file).await?;
        let values = lockdown.get_value(None, None).await?;
        let get = |key: &str| {
            values
                .as_dictionary()
                .and_then(|v| v.get(key))
                .and_then(|v| v.as_string())
                .map(str::to_owned)
        };
        if !get("UniqueDeviceID").is_some_and(|actual| same(&actual, id)) {
            return Err(DeviceError::DeviceIdMismatch);
        }
        let device = DeviceSummary {
            device_id: id.into(),
            udid: Some(id.into()),
            transport: format!("network:{host}"),
            identity_status: IdentityStatus::Available,
            name: get("DeviceName"),
            product_type: get("ProductType"),
            product_version: get("ProductVersion"),
            build_version: get("BuildVersion"),
        };
        Ok((provider, device))
    })
    .await
    .map_err(|_| DeviceError::Timeout)?
}
async fn candidates() -> Result<Vec<IpAddr>, DeviceError> {
    let daemon = mdns_sd::ServiceDaemon::new().map_err(|_| DeviceError::WdaUnavailable)?;
    struct Stop(mdns_sd::ServiceDaemon);
    impl Drop for Stop {
        fn drop(&mut self) {
            let _ = self.0.shutdown();
        }
    }
    let _stop = Stop(daemon.clone());
    let rx = daemon
        .browse("_apple-mobdev2._tcp.local.")
        .map_err(|_| DeviceError::WdaUnavailable)?;
    let remote = daemon
        .browse("_remotepairing._tcp.local.")
        .map_err(|_| DeviceError::WdaUnavailable)?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    let mut addresses = std::collections::BTreeSet::new();
    loop {
        let event = tokio::select! {_=tokio::time::sleep_until(deadline)=>break,event=rx.recv_async()=>event,event=remote.recv_async()=>event};
        let Ok(event) = event else {
            break;
        };
        if let mdns_sd::ServiceEvent::ServiceResolved(service) = event {
            for address in &service.addresses {
                let ip = address.to_ip_addr();
                if allowed(ip) && addresses.len() < 8 {
                    addresses.insert(ip);
                }
            }
        }
    }
    Ok(addresses.into_iter().collect())
}
async fn resolve() -> Result<(TcpProvider, DeviceSummary), DeviceError> {
    let d = DIRECT
        .get()
        .ok_or_else(|| DeviceError::MissingDevice("Wi-Fi".into()))?;
    let pair = pair(d).await?;
    let known = d.host.or(*d.resolved.lock().await);
    if let Some(host) = known {
        match verify(host, &d.id, pair.clone()).await {
            Ok(result) => return Ok(result),
            Err(error) if d.host.is_some() => return Err(error),
            Err(_) => *d.resolved.lock().await = None,
        }
    }
    let mut tasks = tokio::task::JoinSet::new();
    for host in candidates().await? {
        let id = d.id.clone();
        let pair = pair.clone();
        tasks.spawn(async move { verify(host, &id, pair).await });
    }
    while let Some(result) = tasks.join_next().await {
        if let Ok(Ok((provider, summary))) = result {
            *d.resolved.lock().await = Some(provider.addr);
            return Ok((provider, summary));
        }
    }
    Err(DeviceError::MissingDevice(d.id.clone()))
}
pub async fn summary() -> Result<DeviceSummary, DeviceError> {
    resolve().await.map(|(_, device)| device)
}
pub(crate) async fn provider(id: &str) -> Result<Box<dyn IdeviceProvider>, DeviceError> {
    if let Some(d) = DIRECT.get() {
        if !same(&d.id, id) {
            return Err(DeviceError::DeviceIdMismatch);
        }
        return resolve()
            .await
            .map(|(provider, _)| Box::new(provider) as Box<dyn IdeviceProvider>);
    }
    let mut mux = UsbmuxdAddr::from_env_var()?.connect(0).await?;
    let device = crate::wifi::selected_device(&mut mux, id).await?;
    Ok(Box::new(super::provider(device.device_id, device.udid)?))
}
