//! Read-only discovery from an explicitly configured local tunneld HTTP origin.
//! The public GET / contract supplies endpoints; device RSD identity is verified separately.
use super::*;
use rust_ios_device_api::{CheckStatus, DeviceSummary, DiagnosticCheck, DiagnosticReport};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct Discovery {
    http: reqwest::Client,
    origin: url::Url,
    device: String,
}
pub struct Target {
    pub host: IpAddr,
    pub port: u16,
    pub expected_rsd_id: Option<String>,
    pub identity: Option<DeviceSummary>,
}
impl Discovery {
    pub fn new(origin: &str, device: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(
            canonical_udid(device).is_some(),
            "tunneld discovery requires a hardware UDID"
        );
        let origin = url::Url::parse(origin)?;
        let local = match origin.host() {
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            _ => false,
        };
        anyhow::ensure!(
            origin.scheme() == "http"
                && local
                && origin.username().is_empty()
                && origin.password().is_none()
                && origin.path() == "/"
                && origin.query().is_none()
                && origin.fragment().is_none(),
            "tunneld discovery requires a bare HTTP loopback IP origin"
        );
        Ok(Self {
            origin,
            device: device.into(),
            http: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(1))
                .timeout(Duration::from_secs(2))
                .build()?,
        })
    }
    pub async fn resolve(&self) -> Result<Target, &'static str> {
        let mut response = self
            .http
            .get(self.origin.clone())
            .send()
            .await
            .map_err(|_| "tunneld_unreachable")?;
        if !response.status().is_success() {
            return Err("tunneld_http_failed");
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "tunneld_response_lost")?
        {
            if bytes.len() + chunk.len() > 65536 {
                return Err("tunneld_response_too_large");
            }
            bytes.extend_from_slice(&chunk);
        }
        let listing: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "tunneld_invalid_listing")?;
        let object = listing.as_object().ok_or("tunneld_invalid_listing")?;
        let wanted = canonical_udid(&self.device);
        let mut entries = Vec::new();
        for (id, value) in object {
            if canonical_udid(id) == wanted {
                entries.extend(value.as_array().ok_or("tunneld_invalid_listing")?.iter());
            }
        }
        if entries.len() > 16 {
            return Err("tunneld_ambiguous_target");
        }
        if entries.is_empty() {
            return Err("tunneld_device_not_found");
        }
        let mut endpoints = std::collections::BTreeSet::new();
        for entry in entries {
            // Federated endpoints may not be locally routable; never follow an origin URL.
            if entry.get("origin").is_some_and(|value| !value.is_null()) {
                return Err("tunneld_remote_origin_unsupported");
            }
            let host: IpAddr = entry
                .get("tunnel-address")
                .and_then(serde_json::Value::as_str)
                .ok_or("tunneld_invalid_endpoint")?
                .parse()
                .map_err(|_| "tunneld_invalid_endpoint")?;
            let port = entry
                .get("tunnel-port")
                .and_then(serde_json::Value::as_u64)
                .and_then(|p| u16::try_from(p).ok())
                .filter(|p| *p != 0)
                .ok_or("tunneld_invalid_endpoint")?;
            if !is_allowed_rsd_ip(host) {
                return Err("tunneld_endpoint_rejected");
            }
            endpoints.insert((host, port));
        }
        if endpoints.len() != 1 {
            return Err("tunneld_ambiguous_target");
        }
        let (host, port) = endpoints
            .into_iter()
            .next()
            .ok_or("tunneld_device_not_found")?;
        let mut identity = rsd_device(&host.to_string(), port)
            .await
            .map_err(|_| "tunneld_rsd_handshake_failed")?;
        if identity.udid.as_deref().and_then(canonical_udid) != wanted {
            return Err("tunneld_device_identity_mismatch");
        }
        let expected_rsd_id = identity.device_id.clone();
        // Keep the USB UDID stable across transports; still pin each action's RSD handshake UUID.
        identity.device_id = self.device.clone();
        Ok(Target {
            host,
            port,
            expected_rsd_id: Some(expected_rsd_id),
            identity: Some(identity),
        })
    }
    pub async fn augment_diagnostics(&self, report: &mut DiagnosticReport) {
        report
            .checks
            .retain(|check| check.code != "rsd_not_configured");
        let start = Instant::now();
        let result = self.resolve().await;
        let (status, code, message, hints) = match result {
            Ok(target) => {
                if let Some(device) = target.identity {
                    if !report
                        .devices
                        .iter()
                        .any(|d| d.device_id == device.device_id)
                    {
                        report.devices.push(device);
                    }
                }
                if report.health == "unavailable" {
                    report.health = "degraded".into();
                }
                (
                    CheckStatus::Pass,
                    "tunneld_identity_verified",
                    "Endpoint atual descoberto no serviço local e identidade RSD conferida com o UDID autorizado.",
                    vec![],
                )
            }
            Err(code) => {
                if report.health == "ready" {
                    report.health = "degraded".into();
                }
                (CheckStatus::Warn,code,"A descoberta RSD local não confirmou um endpoint para este aparelho.",
                    vec!["Conferir tunneld no loopback do mesmo host e um túnel já pareado/ativo. A descoberta não inicia pareamento nem monta túneis.".into()])
            }
        };
        let elapsed = start.elapsed().as_millis() as u64;
        report.elapsed_ms += elapsed;
        report.checks.push(DiagnosticCheck {
            layer: "transport.rsd_discovery".into(),
            status,
            code: code.into(),
            message: message.into(),
            elapsed_ms: elapsed,
            remediation: hints,
        });
    }
}
fn canonical_udid(value: &str) -> Option<String> {
    if !value.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return None;
    }
    let normalized = value.replace('-', "").to_ascii_uppercase();
    matches!(normalized.len(), 24 | 40).then_some(normalized)
}
impl AppState {
    pub async fn rsd_target(&self) -> Option<Target> {
        if let Some(discovery) = &self.rsd_discovery {
            return match discovery.resolve().await {
                Ok(target) => Some(target),
                Err(code) => {
                    tracing::warn!(code, "local RSD discovery failed before dispatch");
                    None
                }
            };
        }
        self.rsd.map(|(host, port)| Target {
            host,
            port,
            expected_rsd_id: None,
            identity: None,
        })
    }
    pub fn has_rsd(&self) -> bool {
        self.rsd.is_some() || self.rsd_discovery.is_some()
    }
}
