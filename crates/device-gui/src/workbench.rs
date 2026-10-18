use super::*;
use super::{components, icons::Icon, theme};
use rust_ios_device_api::{DiagnosticCheck, TransportHistory, UsbDevice, UsbRecoveryTarget};

impl Studio {
    pub(super) fn render_workbench(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.page != 2 {
            self.render_remote(ui, ctx);
            return;
        }
        let log = self
            .local_connection
            .as_ref()
            .map(|connection| connection.log.display().to_string());
        let Some(report) = self.report.clone() else {
            components::card(ui, |ui| {
                components::empty_state(
                    ui,
                    Icon::Activity,
                    "O diagnóstico aparece aqui",
                    "Consulte o computador para verificar hardware USB, serviço do sistema, usbmux, Lockdown e RSD. Nenhuma captura é feita automaticamente.",
                );
                connection_log(ui, ctx, log.as_deref());
            });
            return;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        summary(ui, ctx, &report, self.report_label, log.as_deref(), now);
        components::gap(ui, theme::SPACE_24);
        checks(ui, &report.checks);
        if let Some(history) = &report.transport_history {
            components::gap(ui, theme::SPACE_16);
            connection_history(ui, history, now);
        }
        if report.platform == "windows" {
            components::gap(ui, theme::SPACE_24);
            self.apple_interfaces(ui, ctx, report.usb_recovery_targets);
        }
        components::gap(ui, theme::SPACE_24);
        usb_inventory(ui, &report.usb_devices);
    }

    fn apple_interfaces(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        targets: Vec<UsbRecoveryTarget>,
    ) {
        components::section_heading(ui, Icon::Usb, "Interfaces Apple identificadas");
        if targets.is_empty() {
            components::card(ui, |ui| {
                components::empty_state(
                    ui,
                    Icon::Usb,
                    "Nenhuma interface Apple verificável",
                    "Não há uma interface associada de forma verificável ao aparelho. A recuperação dirigida está indisponível.",
                );
            });
        }
        for target in targets {
            components::card(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Interface do iPhone").strong());
                    ui.label(
                        RichText::new(&target.driver_service)
                            .monospace()
                            .color(theme::MUTED),
                    );
                });
                detail(ui, "Aparelho", &target.device_id);
                let problem = target
                    .problem_code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "desconhecido".into());
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!("Problema: {problem}")).color(theme::MUTED));
                    ui.label(
                        RichText::new(format!(
                            "Energia: {}",
                            target.power_state.as_deref().unwrap_or("desconhecida")
                        ))
                        .color(theme::MUTED),
                    );
                });
                detail(
                    ui,
                    "Pacote do driver",
                    &format!(
                        "{} · versão {} · {}",
                        target.driver_provider.as_deref().unwrap_or("desconhecido"),
                        target.driver_version.as_deref().unwrap_or("desconhecida"),
                        target.driver_inf.as_deref().unwrap_or("INF desconhecido")
                    ),
                );
                ui.label(
                    RichText::new(&target.instance_id)
                        .monospace()
                        .size(theme::TEXT_LABEL)
                        .color(theme::MUTED),
                );
                components::gap(ui, theme::SPACE_8);

                // Preserve the exact original local/daemon authorization and cooldown gates.
                let local = self.local_recovery_available();
                let enabled = !self.busy
                    && if local {
                        self.local_recovery_remaining() == 0
                    } else {
                        self.api_confirmed && self.usb_recovery_enabled
                    };
                let label = if local {
                    "Reiniciar interface neste computador"
                } else {
                    "Reiniciar esta interface USB"
                };
                if components::action(ui, Icon::Refresh, label, enabled, false).clicked() {
                    if local {
                        self.recover_local(
                            local::LocalRecovery::Usb {
                                device_id: target.device_id,
                                instance_id: target.instance_id,
                            },
                            ctx,
                        );
                    } else {
                        self.dispatch(
                            Action::RestartUsb(target.device_id, target.instance_id),
                            ctx,
                        );
                    }
                }
            });
            components::gap(ui, theme::SPACE_8);
        }
        if self.local_recovery_available() {
            muted(
                ui,
                "A recuperação local verifica esta interface novamente e exige administrador. Um resultado reboot_required precisa de intervenção; o computador não é reiniciado automaticamente.",
            );
        } else if !self.usb_recovery_enabled {
            muted(
                ui,
                "Habilite a recuperação USB antes de conectar localmente, ou use --allow-usb-interface-restart no daemon. Exige administrador; nunca reinicia o PC automaticamente.",
            );
        }
    }
}

fn summary(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    report: &DiagnosticReport,
    source: &str,
    log: Option<&str>,
    now: u64,
) {
    let (health, color) = match report.health.as_str() {
        "ready" => ("Pronto", theme::SUCCESS),
        "degraded" => ("Atenção", theme::WARNING),
        _ => ("Indisponível", theme::ERROR),
    };
    components::card(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            badge(ui, health, color);
            ui.label(RichText::new(source).strong());
            ui.label(
                RichText::new(format!("{} / {}", report.platform, report.architecture))
                    .color(theme::MUTED),
            );
        });
        components::gap(ui, theme::SPACE_8);
        ui.separator();
        components::gap(ui, theme::SPACE_8);
        let pid = report
            .service
            .process_id
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "—".into());
        let state = format!("{} · PID {pid}", report.service.state);
        if ui.available_width() >= 560.0 {
            ui.columns(2, |columns| {
                detail(&mut columns[0], "Serviço do sistema", &report.service.name);
                detail(&mut columns[1], "Estado / processo", &state);
            });
        } else {
            detail(ui, "Serviço do sistema", &report.service.name);
            detail(ui, "Estado / processo", &state);
        }
        components::gap(ui, theme::SPACE_8);
        muted(
            ui,
            &format!(
                "Atualizado há {} s · consulta em {} ms · atualização sob demanda",
                now.saturating_sub(report.generated_at_unix_ms) / 1000,
                report.elapsed_ms
            ),
        );
        connection_log(ui, ctx, log);
    });
}

fn checks(ui: &mut egui::Ui, checks: &[DiagnosticCheck]) {
    components::section_heading(ui, Icon::Scan, "Verificações por camada");
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("{} verificações", checks.len())).color(theme::MUTED));
        for (status, label, color) in [
            (CheckStatus::Pass, "OK", theme::SUCCESS),
            (CheckStatus::Warn, "avisos", theme::WARNING),
            (CheckStatus::Fail, "falhas", theme::ERROR),
        ] {
            let count = checks.iter().filter(|check| check.status == status).count();
            if count > 0 {
                badge(ui, &format!("{count} {label}"), color);
            }
        }
    });
    components::gap(ui, theme::SPACE_8);
    if checks.is_empty() {
        components::card(ui, |ui| {
            components::empty_state(
                ui,
                Icon::Scan,
                "Sem verificações neste relatório",
                "Uma nova consulta preencherá o estado de cada camada da conexão.",
            );
        });
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("diagnostic-check-cards")
        .max_height(360.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for check in checks {
                let (status, color) = match check.status {
                    CheckStatus::Pass => ("OK", theme::SUCCESS),
                    CheckStatus::Warn => ("AVISO", theme::WARNING),
                    CheckStatus::Fail => ("FALHA", theme::ERROR),
                    CheckStatus::Unknown => ("NÃO VERIFICADO", theme::MUTED),
                    CheckStatus::Skipped => ("IGNORADO", theme::MUTED),
                };
                components::card(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        badge(ui, status, color);
                        ui.label(RichText::new(&check.layer).strong());
                        muted(ui, &format!("{} ms", check.elapsed_ms));
                    });
                    ui.label(RichText::new(&check.code).monospace().color(theme::MUTED));
                    ui.label(&check.message);
                    if !check.remediation.is_empty() {
                        components::gap(ui, theme::SPACE_4);
                        ui.label(
                            RichText::new("Próximos passos")
                                .size(theme::TEXT_LABEL)
                                .strong(),
                        );
                        for hint in &check.remediation {
                            muted(ui, &format!("• {hint}"));
                        }
                    }
                });
                components::gap(ui, theme::SPACE_8);
            }
        });
}

fn connection_history(ui: &mut egui::Ui, history: &TransportHistory, now: u64) {
    components::card(ui, |ui| {
        egui::CollapsingHeader::new(format!(
            "Histórico da conexão · {} consultas · {} registros recentes",
            history.samples, history.observations.len()))
            .id_salt("diagnostic-transport-history")
            .show(ui, |ui| {
                components::gap(ui, theme::SPACE_8);
                muted(ui, &format!(
                    "Enumeração usbmux a cada {} s ou mais. Última amostra há {} s. Mantém até 32 transições, sem conteúdo de tela.",
                    history.sample_interval_ms / 1000,
                    now.saturating_sub(history.last_sample_at_unix_ms) / 1000));
                components::gap(ui, theme::SPACE_8);
                for item in history.observations.iter().rev() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(format!("há {} s",
                            now.saturating_sub(item.observed_at_unix_ms) / 1000)).color(theme::MUTED));
                        ui.label(RichText::new(&item.code).monospace());
                        ui.label(RichText::new(format!("{} aparelho(s)",
                            item.device_count)).color(theme::MUTED));
                    });
                }
                components::gap(ui, theme::SPACE_8);
                muted(ui, "São instantes observados por consulta; não provam o momento físico da desconexão nem sua causa.");
            });
    });
}

fn usb_inventory(ui: &mut egui::Ui, devices: &[UsbDevice]) {
    components::card(ui, |ui| {
        egui::CollapsingHeader::new(format!("Inventário USB · {} dispositivos", devices.len()))
            .id_salt("diagnostic-usb-inventory")
            .show(ui, |ui| {
                components::gap(ui, theme::SPACE_8);
                if devices.is_empty() {
                    ui.label(RichText::new("Nenhum dispositivo USB retornado por este inventário.")
                        .color(theme::MUTED));
                }
                egui::ScrollArea::vertical()
                    .id_salt("diagnostic-usb-devices")
                    .max_height(220.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for (index, device) in devices.iter().enumerate() {
                            egui::CollapsingHeader::new(format!("{:04x}:{:04x} · {}",
                                device.vendor_id, device.product_id,
                                device.name.as_deref().unwrap_or("USB")))
                                .id_salt(("diagnostic-usb-device", index))
                                .show(ui, |ui| {
                                    detail(ui, "Barramento / endereço", &device.location);
                                    if device.interfaces.is_empty() {
                                        muted(ui, "O driver não expôs descritores de interfaces neste inventário.");
                                    }
                                    for interface in &device.interfaces {
                                        ui.monospace(format!("Interface {} · classe {:02x}/{:02x}/{:02x}",
                                            interface.number, interface.class, interface.subclass, interface.protocol));
                                    }
                                });
                        }
                    });
            });
    });
}

fn detail(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = theme::SPACE_4;
        muted(ui, label);
        ui.label(value);
    });
}

fn muted(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(theme::TEXT_LABEL)
            .color(theme::MUTED),
    );
}

fn badge(ui: &mut egui::Ui, label: &str, color: Color32) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.12))
        .corner_radius(theme::RADIUS_CARD)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.label(
                RichText::new(label)
                    .size(theme::TEXT_LABEL)
                    .strong()
                    .color(color),
            );
        });
}

fn connection_log(ui: &mut egui::Ui, ctx: &egui::Context, path: Option<&str>) {
    let Some(path) = path else {
        return;
    };
    components::gap(ui, theme::SPACE_8);
    egui::CollapsingHeader::new("Log da conexão local")
        .id_salt("diagnostic-local-log")
        .show(ui, |ui| {
            components::gap(ui, theme::SPACE_4);
            ui.label(
                RichText::new(path)
                    .monospace()
                    .size(theme::TEXT_LABEL)
                    .color(theme::MUTED),
            );
            if components::action(ui, Icon::Blocks, "Copiar caminho do log", true, false).clicked()
            {
                ctx.copy_text(path.to_owned());
            }
        });
}
