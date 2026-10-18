use super::*;
use super::{components, icons::Icon, theme};

impl Studio {
    pub(super) fn render_elements(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        components::page_header(
            ui,
            "AUTOMAÇÃO",
            "Elementos da interface",
            "Inspecione controles, crie seletores e acompanhe o resultado das suas ações.",
        );
        if self.local_connection.as_ref().is_some_and(|c| c.mode == 1) {
            components::card(ui, |ui| {
                components::section_heading(ui, Icon::Scan, "Automação por elementos");
                ui.label("Nesta sessão Wi-Fi, use a tela ao vivo e os controles por gesto. A automação por elementos precisa do runner WDA e do transporte compatível.");
                components::gap(ui, theme::SPACE_16);
                if components::action(ui, Icon::Monitor, "Abrir tela ao vivo", !self.busy, true)
                    .clicked()
                {
                    self.open_remote_window(ctx);
                }
            });
            return;
        }
        if !self.api_confirmed || self.selected.is_empty() {
            components::card(ui, |ui| {
                components::empty_state(
                    ui,
                    Icon::Scan,
                    "Conecte um aparelho para inspecionar",
                    "Os controles da tela aparecerão aqui depois de abrir uma sessão e consultar os elementos.",
                );
                if components::action(ui, Icon::Plug, "Ir para Conectar", true, true).clicked() {
                    self.page = 3;
                }
            });
            return;
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !self.busy && !self.selected.is_empty(),
                    egui::Button::new("Consultar elementos")
                        .fill(theme::SELECTED)
                        .min_size(egui::vec2(0.0, theme::BUTTON_PRIMARY)),
                )
                .clicked()
            {
                self.dispatch(Action::Elements(self.selected.clone()), ctx);
            }
            ui.add(
                egui::TextEdit::singleline(&mut self.element_filter)
                    .hint_text("Filtrar por texto, identificador ou tipo")
                    .desired_width(ui.available_width().min(330.0)),
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.named_only, "Com nome ou identificador");
            ui.checkbox(&mut self.visible_only, "Somente visíveis");
        });
        let Some(snapshot) = self.elements.clone() else {
            components::gap(ui, theme::SPACE_24);
            components::card(ui, |ui| {
                components::empty_state(
                    ui,
                    Icon::Scan,
                    "Pronto para explorar a tela",
                    "Consulte os elementos para ver nomes, identificadores e ações disponíveis.",
                );
                ui.small("WDA oferece leitura e ativação com um runner assinado. AX é uma alternativa somente para inspeção e pode rolar a tela.");
            });
            ui.collapsing("Integração com IA", |ui| {
                ui.label("MCP: ui_snapshot → find_element → activate_element → ui_snapshot.");
                ui.label("Seletores ambíguos e referências expiradas são recusados. Uma ação incerta exige nova observação.");
            });
            return;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let age = now.saturating_sub(snapshot.created_at_unix_ms);
        let fresh = age < 30_000 && snapshot.device_id == self.selected;
        ctx.request_repaint_after(Duration::from_secs(1));
        ui.horizontal(|ui| {
            ui.colored_label(
                ACCENT,
                format!(
                    "{} · {} elementos",
                    snapshot.provider,
                    snapshot.elements.len()
                ),
            );
            ui.small(format!(
                "há {} s · {}",
                age / 1000,
                if snapshot.complete {
                    "consulta concluída"
                } else {
                    "consulta parcial"
                }
            ));
            if !fresh {
                ui.colored_label(theme::WARNING, "Atualize para ativar");
            }
        });
        for warning in &snapshot.warnings {
            ui.small(warning);
        }
        if let Some(app) = &snapshot.app_bundle_id {
            ui.small(format!(
                "App: {app} · áreas em {}",
                snapshot
                    .bounds_units
                    .as_deref()
                    .unwrap_or("unidade desconhecida")
            ));
        }
        if !snapshot.activation_supported {
            ui.colored_label(
                theme::WARNING,
                "Somente inspeção. Inicie o runner WDA para habilitar ativação.",
            );
        }
        ui.separator();
        let filter = self.element_filter.to_lowercase();
        let filtered: Vec<_> = snapshot
            .elements
            .iter()
            .filter(|element| {
                (!self.named_only || !element.label.is_empty() || element.identifier.is_some())
                    && (!self.visible_only || element.visible != Some(false))
            })
            .filter(|element| {
                format!(
                    "{} {} {}",
                    element.label,
                    element.identifier.as_deref().unwrap_or(""),
                    element.role.as_deref().unwrap_or("")
                )
                .to_lowercase()
                .contains(&filter)
            })
            .collect();
        egui::ScrollArea::vertical()
            .id_salt("elements")
            .max_height((ctx.content_rect().height() * 0.32).clamp(120.0, 360.0))
            .show_rows(
                ui,
                if ctx.content_rect().width() < 1000.0 {
                    136.0
                } else {
                    112.0
                },
                filtered.len(),
                |ui, rows| {
                    for index in rows {
                        let element = filtered[index];
                        let caption = if element.label.is_empty() {
                            element.identifier.as_deref().unwrap_or("Sem rótulo")
                        } else {
                            &element.label
                        };
                        egui::Frame::new()
                            .fill(theme::SURFACE)
                            .stroke(egui::Stroke::new(1.0, theme::BORDER))
                            .corner_radius(theme::RADIUS_CARD)
                            .inner_margin(12)
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                if ui
                                    .add(
                                        egui::Button::selectable(
                                            self.selected_element == element.element_id,
                                            caption.chars().take(85).collect::<String>(),
                                        )
                                        .wrap(),
                                    )
                                    .on_hover_text(caption)
                                    .clicked()
                                {
                                    self.selected_element = element.element_id.clone();
                                }
                                ui.small(format!(
                                    "{} · {} · {} · nível {}",
                                    element.role.as_deref().unwrap_or("Tipo desconhecido"),
                                    match element.enabled {
                                        Some(true) => "habilitado",
                                        Some(false) => "desabilitado",
                                        None => "estado desconhecido",
                                    },
                                    match element.visible {
                                        Some(true) => "visível",
                                        Some(false) => "fora da tela",
                                        None => "visibilidade desconhecida",
                                    },
                                    element.depth
                                ));
                                if let Some(identifier) = &element.identifier {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(identifier).monospace(),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(identifier);
                                }
                            });
                    }
                },
            );
        ui.separator();
        if let Some(element) = snapshot
            .elements
            .iter()
            .find(|e| e.element_id == self.selected_element)
        {
            ui.strong("Elemento selecionado");
            ui.add(egui::Label::new(&element.label).truncate())
                .on_hover_text(&element.label);
            if !element.actions.iter().any(|action| action == "activate") {
                ui.small("Este nó não expõe ativação suportada. Selecione um controle interativo.");
            }
            ui.small(format!("Referência: {}", element.element_id));
            if let Some(value) = &element.value {
                ui.add(egui::Label::new(format!("Valor: {value}")).truncate());
            }
            if let Some(bounds) = &element.bounds {
                ui.small(format!(
                    "Área: ({:.0}, {:.0}) · {:.0} × {:.0}",
                    bounds.x, bounds.y, bounds.width, bounds.height
                ));
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        !self.busy
                            && fresh
                            && snapshot.activation_supported
                            && element.actions.iter().any(|action| action == "activate")
                            && element.enabled != Some(false)
                            && element.visible != Some(false),
                        egui::Button::new("Ativar elemento")
                            .fill(theme::SELECTED)
                            .min_size(egui::vec2(0.0, theme::BUTTON_PRIMARY)),
                    )
                    .clicked()
                {
                    self.dispatch(
                        Action::Activate(
                            self.selected.clone(),
                            snapshot.snapshot_id.clone(),
                            element.element_id.clone(),
                        ),
                        ctx,
                    );
                }
                if ui.button("Copiar seletor JSON").clicked() {
                    let selector = if let Some(id) = &element.identifier {
                        serde_json::json!({"identifier":id})
                    } else {
                        serde_json::json!({"label":element.label,"role":element.role})
                    };
                    ctx.copy_text(selector.to_string());
                }
            });
            ui.weak("Após ativar, uma nova consulta permite verificar a mudança. A confirmação do WDA indica envio da ação.");
        } else {
            ui.weak("Selecione um elemento para ver detalhes, copiar o seletor ou ativá-lo.");
        }
    }
}
