//! Read-only probes and bounded recovery of the host's known iOS multiplexer service.
pub mod service;
pub mod usb_recovery;

use idevice::usbmuxd::UsbmuxdAddr;
use nusb::MaybeFuture;
use rust_ios_device_api::*;
use std::{
    net::IpAddr,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn ms(start: Instant) -> u64 {
    start.elapsed().as_millis().min(u64::MAX as u128) as u64
}

fn check(
    layer: &str,
    status: CheckStatus,
    code: &str,
    message: &str,
    elapsed_ms: u64,
    remediation: &[&str],
) -> DiagnosticCheck {
    DiagnosticCheck {
        layer: layer.into(),
        status,
        code: code.into(),
        message: message.into(),
        elapsed_ms,
        remediation: remediation.iter().map(|s| (*s).into()).collect(),
    }
}

async fn inventory() -> Result<Vec<UsbDevice>, ()> {
    tokio::time::timeout(
        Duration::from_secs(4),
        tokio::task::spawn_blocking(|| {
            nusb::list_devices()
                .wait()
                .map(|devices| {
                    devices
                        .map(|device| UsbDevice {
                            vendor_id: device.vendor_id(),
                            product_id: device.product_id(),
                            name: device.product_string().map(str::to_owned),
                            manufacturer: device.manufacturer_string().map(str::to_owned),
                            location: format!("{}:{}", device.bus_id(), device.device_address()),
                            interfaces: device
                                .interfaces()
                                .map(|interface| UsbInterface {
                                    number: interface.interface_number(),
                                    class: interface.class(),
                                    subclass: interface.subclass(),
                                    protocol: interface.protocol(),
                                })
                                .collect(),
                        })
                        .collect()
                })
                .map_err(|_| ())
        }),
    )
    .await
    .map_err(|_| ())?
    .map_err(|_| ())?
}

fn mux_empty_check(apple_usb_present: bool) -> DiagnosticCheck {
    if apple_usb_present {
        check(
            "transport.usbmux",
            CheckStatus::Warn,
            "apple_usb_present_mux_empty",
            "O sistema lista hardware USB Apple, mas o multiplexador não reportou aparelhos. Isso não prova que o hardware Apple seja o iPhone nem que o serviço esteja travado.",
            0,
            &[
                "Conferir o dispositivo e as interfaces Apple no inventário USB.",
                "Se for o iPhone, verificar driver/pareamento; reiniciar o serviço é uma opção explícita de recuperação.",
            ],
        )
    } else {
        check(
            "transport.usbmux",
            CheckStatus::Warn,
            "mux_empty",
            "O serviço respondeu, mas não reportou aparelhos; a causa física não foi determinada.",
            0,
            &[
                "Verificar conexão, desbloqueio e confiança no computador.",
                "Comparar o inventário USB e o estado do serviço antes de reiniciá-lo.",
            ],
        )
    }
}

fn health(devices: &[DeviceSummary], checks: &[DiagnosticCheck]) -> String {
    if !devices
        .iter()
        .any(|d| matches!(d.identity_status, IdentityStatus::Available))
    {
        return "unavailable".into();
    }
    if checks.iter().any(|c| {
        matches!(
            c.status,
            CheckStatus::Warn | CheckStatus::Fail | CheckStatus::Unknown
        )
    }) {
        "degraded"
    } else {
        "ready"
    }
    .into()
}

fn managed_endpoint_for(platform: &str, configured: Option<&str>) -> bool {
    match (platform, configured) {
        ("windows" | "linux", None) => true,
        ("windows", Some("127.0.0.1:27015")) => true,
        ("linux", Some("/var/run/usbmuxd" | "/run/usbmuxd")) => true,
        _ => false,
    }
}

pub fn managed_endpoint() -> bool {
    managed_endpoint_for(
        std::env::consts::OS,
        std::env::var("USBMUXD_SOCKET_ADDRESS").ok().as_deref(),
    )
}

pub async fn diagnose(
    rsd: Option<(IpAddr, u16)>,
    allowed_device: Option<&str>,
    recovery_enabled: bool,
) -> DiagnosticReport {
    let recovery_enabled = recovery_enabled && managed_endpoint();
    let start = Instant::now();
    let generated_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let (usb_result, service_result) = tokio::join!(
        inventory(),
        tokio::time::timeout(Duration::from_secs(4), service::snapshot())
    );
    let service = service_result.unwrap_or(ServiceInfo {
        name: service::service_name().into(),
        state: "unknown".into(),
        process_id: None,
        code: "service_query_timeout".into(),
    });
    let mut checks = Vec::new();
    let usb_devices = match usb_result {
        Ok(devices) => {
            checks.push(check("host.usb", CheckStatus::Pass, "usb_inventory_available", "Inventário USB obtido sem abrir nem reivindicar interfaces; drivers podem omitir descritores.", ms(start), &[]));
            devices
        }
        Err(()) => {
            checks.push(check(
                "host.usb",
                CheckStatus::Unknown,
                "usb_inventory_unavailable",
                "O inventário USB falhou ou excedeu o limite de consulta.",
                ms(start),
                &["Verificar permissões e suporte de enumeração USB do sistema."],
            ));
            Vec::new()
        }
    };
    let running = matches!(service.state.as_str(), "running" | "active");
    checks.push(check("host.service", if running { CheckStatus::Pass } else if service.state == "unknown" { CheckStatus::Unknown } else { CheckStatus::Warn },
        &service.code, if running { "O gerenciador do sistema informa que o serviço está ativo. A resposta do protocolo é verificada separadamente." } else { "O serviço não foi confirmado ativo pelo gerenciador do sistema." }, ms(start),
        if running { &[] } else { &["Consultar o gerenciador de serviços do host; verificar instalação e privilégios."] }));
    let probe_start = Instant::now();
    let mux = tokio::time::timeout(Duration::from_secs(3), async {
        let address = UsbmuxdAddr::from_env_var()?;
        let mut mux = address.connect(0).await?;
        mux.get_devices()
            .await
            .map_err(rust_ios_device_core::DeviceError::from)
    })
    .await;
    let mut devices = Vec::new();
    match mux {
        Ok(Ok(raw)) if raw.is_empty() => {
            let mut finding = mux_empty_check(usb_devices.iter().any(|d| d.vendor_id == 0x05ac));
            finding.elapsed_ms = ms(probe_start);
            checks.push(finding);
        }
        Ok(Ok(_)) => {
            checks.push(check(
                "transport.usbmux",
                CheckStatus::Pass,
                "mux_protocol_ok",
                "O serviço respondeu à enumeração usbmux.",
                ms(probe_start),
                &[],
            ));
            if rust_ios_device_core::wireless::id().is_none() {
                match rust_ios_device_core::list_devices().await {
                Ok(found) => {
                    for device in found.iter().filter(|d| allowed_device.is_none_or(|id| d.device_id == id)) {
                        let (status, code, message, hint): (_, _, _, &[&str]) = match device.identity_status {
                            IdentityStatus::Available => (CheckStatus::Pass, "identity_available", "A leitura de identidade Lockdown respondeu; outros serviços ainda podem falhar.", &[]),
                            IdentityStatus::Locked => (CheckStatus::Warn, "device_locked", "O protocolo informou bloqueio do aparelho.", &["Desbloquear o aparelho; reiniciar o serviço não remove esse bloqueio."]),
                            IdentityStatus::Unpaired => (CheckStatus::Warn, "pairing_required", "O protocolo informou identidade de host inválida ou pareamento ausente.", &["Conectar por USB e aceitar a confiança se solicitada; não apagar os registros de pareamento automaticamente."]),
                            IdentityStatus::ServiceUnavailable => (CheckStatus::Warn, "identity_unavailable", "O aparelho foi enumerado, mas a identidade não pôde ser lida.", &["Verificar desbloqueio, pareamento e disponibilidade do serviço Lockdown."]),
                        };
                        checks.push(check("device.lockdown", status, code, message, 0, hint));
                    }
                    devices.extend(found);
                }
                Err(_) => checks.push(check("device.lockdown", CheckStatus::Fail, "identity_probe_failed", "A enumeração usbmux respondeu, mas a consulta de identidade falhou ou excedeu o tempo.", ms(probe_start), &["Comparar enumeração e identidade; considerar nova sessão ou recuperação do serviço."])),
            }
            }
        }
        Ok(Err(error)) => {
            let code = match error.io_kind() {
                Some(std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound) => {
                    "mux_endpoint_unavailable"
                }
                Some(std::io::ErrorKind::PermissionDenied) => "mux_permission_denied",
                _ => "mux_protocol_failed",
            };
            checks.push(check(
                "transport.usbmux",
                CheckStatus::Fail,
                code,
                "Não foi possível completar a enumeração no endpoint usbmux configurado.",
                ms(probe_start),
                &["Conferir USBMUXD_SOCKET_ADDRESS, o estado do serviço e as permissões do host."],
            ));
        }
        Err(_) => checks.push(check(
            "transport.usbmux",
            CheckStatus::Fail,
            "mux_probe_timeout",
            "O endpoint usbmux não completou a enumeração em 3 segundos.",
            ms(probe_start),
            &["Repetir diagnóstico; se persistir, considerar reinício explícito do serviço."],
        )),
    }
    if rust_ios_device_core::wireless::id().is_some() {
        let wifi_started = Instant::now();
        match tokio::time::timeout(
            Duration::from_secs(15),
            rust_ios_device_core::wireless::summary(),
        )
        .await
        {
            Ok(Ok(device)) => {
                checks.push(check("transport.wifi",CheckStatus::Pass,"wifi_lan_identity_verified","Conexão TCP direta pela rede com TLS do pareamento e identidade do iPhone conferidos. Nenhum transporte USB foi usado para conectar ao aparelho.",ms(wifi_started),&[]));
                devices.retain(|d| d.device_id != device.device_id);
                devices.push(device);
            }
            result => {
                let detail = match result {
                    Ok(Err(error)) => error.to_string(),
                    _ => "prazo de conexão excedido".into(),
                };
                checks.push(check("transport.wifi",CheckStatus::Fail,"wifi_lan_unavailable",&format!("A conexão direta pela rede não foi confirmada: {detail}"),ms(wifi_started),&["Verificar o UDID pareado, Wi-Fi habilitado no iPhone, mesma rede e isolamento entre clientes. USB é necessário para preparar um pareamento ausente, não para manter a sessão de rede."]));
            }
        }
    }
    if let Some((host, port)) = rsd {
        let rsd_start = Instant::now();
        match rust_ios_device_core::rsd_device(&host.to_string(), port).await {
            Ok(device) => {
                checks.push(check(
                    "transport.rsd",
                    CheckStatus::Pass,
                    "rsd_handshake_ok",
                    "O túnel completou o handshake RSD; abrir TCP sozinho não comprova isso.",
                    ms(rsd_start),
                    &[],
                ));
                devices.push(device);
            }
            Err(_) => checks.push(check(
                "transport.rsd",
                CheckStatus::Fail,
                "rsd_handshake_failed",
                "O endpoint configurado não completou o handshake RSD.",
                ms(rsd_start),
                &[
                    "Conferir se o túnel está ativo e o endereço/porta ainda são os atuais.",
                    "Compartilhar uma identidade de host estável entre clientes do mesmo túnel.",
                ],
            )),
        }
    } else {
        checks.push(check("transport.rsd", CheckStatus::Skipped, "rsd_not_configured", "Nenhum endpoint RSD de rede foi configurado; USB continua disponível independentemente.", 0, &[]));
    }
    if let Some(allowed) = allowed_device {
        devices.retain(|d| d.device_id == allowed);
    }
    let usb_recovery_targets = match usb_recovery::inventory().await {
        Ok(targets) => {
            let targets: Vec<_> = targets
                .into_iter()
                .filter(|target| allowed_device.is_none_or(|id| target.device_id == id))
                .collect();
            if cfg!(windows) {
                checks.push(check("host.usb_interface", if targets.is_empty() {CheckStatus::Unknown} else {CheckStatus::Pass},
                    if targets.is_empty() {"apple_usb_mapping_unavailable"} else {"apple_usb_mapping_verified"},
                    "Associação por dispositivo USB Apple presente, serial do pai e interface filha com driver conhecido; não indica disponibilidade do usbmux.", 0, &[]));
                for target in &targets {
                    if target.problem_code.is_some_and(|code| code != 0) {
                        checks.push(check("host.usb_interface", CheckStatus::Warn, "apple_usb_problem_code", &format!("A interface Apple reportou código PnP {}.",target.problem_code.unwrap()), 0,
                            &["Conferir a interface identificada no relatório; não substituir drivers automaticamente."]));
                    }
                }
            }
            targets
        }
        Err(_) => {
            checks.push(check(
                "host.usb_interface",
                CheckStatus::Unknown,
                "apple_usb_mapping_failed",
                "Não foi possível verificar a associação da interface USB ao aparelho.",
                0,
                &[],
            ));
            Vec::new()
        }
    };
    DiagnosticReport {
        api_version: API_VERSION,
        generated_at_unix_ms,
        elapsed_ms: ms(start),
        platform: std::env::consts::OS.into(),
        architecture: std::env::consts::ARCH.into(),
        health: health(&devices, &checks),
        recovery_enabled,
        service,
        checks,
        devices,
        usb_devices,
        usb_recovery_targets,
        transport_history: None,
    }
}

pub async fn recover(rsd: Option<(IpAddr, u16)>, allowed_device: Option<&str>) -> RecoveryReport {
    let start = Instant::now();
    let before = service::snapshot().await;
    let result = if managed_endpoint() {
        service::restart().await
    } else {
        Err("recovery_endpoint_not_managed".into())
    };
    let after = service::snapshot().await;
    let code = match result {
        Ok(()) if matches!(after.state.as_str(), "running" | "active") => {
            "service_restart_completed".into()
        }
        Ok(()) => "service_not_running_after_restart".into(),
        Err(code) => code,
    };
    let outcome = if code == "service_restart_completed" {
        "completed"
    } else {
        "failed"
    }
    .into();
    let mut diagnostics = diagnose(rsd, allowed_device, true).await;
    if code == "service_restart_completed" && diagnostics.health == "unavailable" {
        diagnostics.checks.push(check(
            "recovery.verification", CheckStatus::Warn, "service_restarted_device_unavailable",
            "O serviço reiniciou, mas nenhum aparelho com identidade disponível foi confirmado. O reinício do serviço não comprova recuperação da conexão.",
            ms(start),
            &["Consultar novamente o diagnóstico após a reenumeração; não repetir gestos pendentes.",
              "No Windows, se a interface USB Apple continuar presente, consultar scripts/repair-windows-usb.ps1 para recuperação explícita da interface selecionada."]));
    }
    RecoveryReport {
        action: "restart_multiplexer_service".into(),
        outcome,
        code,
        elapsed_ms: ms(start),
        before,
        after,
        diagnostics,
    }
}

/// Explicit recovery of one freshly verified local interface; a completed PnP operation
/// does not prove phone connectivity. The caller owns serialization and cooldown.
pub async fn recover_usb_interface(
    device_id: &str,
    instance_id: &str,
    rsd: Option<(IpAddr, u16)>,
    service_recovery_enabled: bool,
) -> UsbRecoveryReport {
    let result = usb_recovery::restart(device_id, instance_id).await;
    let (outcome, code) = match result {
        Ok("completed") => ("completed", "usb_interface_restart_completed".to_owned()),
        Ok("reboot_required") => (
            "reboot_required",
            "usb_interface_reboot_required".to_owned(),
        ),
        Ok(_) => ("failed", "usb_restart_failed".to_owned()),
        Err(code) => (
            if code == "usb_restart_outcome_unknown" {
                "unknown"
            } else {
                "failed"
            },
            code,
        ),
    };
    UsbRecoveryReport {
        action: "restart_apple_usb_interface".into(),
        outcome: outcome.into(),
        code,
        instance_id: instance_id.into(),
        diagnostics: diagnose(rsd, Some(device_id), service_recovery_enabled).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_mux_does_not_claim_service_or_hardware_cause() {
        assert_eq!(mux_empty_check(false).code, "mux_empty");
        assert_eq!(mux_empty_check(true).code, "apple_usb_present_mux_empty");
        assert_eq!(mux_empty_check(true).status, CheckStatus::Warn);
    }
    #[test]
    fn healthy_service_without_a_device_is_not_ready() {
        assert_eq!(
            health(
                &[],
                &[check(
                    "host.service",
                    CheckStatus::Pass,
                    "service_queried",
                    "",
                    0,
                    &[]
                )]
            ),
            "unavailable"
        );
    }
    #[test]
    fn restarting_local_service_is_not_a_repair_for_custom_or_remote_endpoint() {
        assert!(managed_endpoint_for("windows", None));
        assert!(managed_endpoint_for("linux", Some("/run/usbmuxd")));
        assert!(!managed_endpoint_for("windows", Some("192.168.0.9:27015")));
        assert!(!managed_endpoint_for("windows", Some("127.0.0.1:9999")));
        assert!(!managed_endpoint_for("linux", Some("127.0.0.1:27015")));
    }
}
