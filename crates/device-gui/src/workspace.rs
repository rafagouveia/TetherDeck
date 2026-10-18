//! Device workspace shell: editor-like navigation, compact chrome and focused task views.
use super::icons::{self, Icon};
use super::*;
use super::{components, theme};
const MUTED: Color32 = theme::MUTED;
fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    components::card(ui, add);
}
fn title(ui: &mut egui::Ui, eyebrow: &str, heading: &str, description: &str) {
    components::page_header(ui, eyebrow, heading, description);
}
fn page_name(page: u8) -> &'static str {
    match page {
        0 => "Elementos",
        1 => "Tela ao vivo",
        2 => "Diagnóstico",
        4 => "Integrações",
        _ => "Conectar",
    }
}
fn page_icon(page: u8) -> Icon {
    match page {
        0 => Icon::Scan,
        1 => Icon::Monitor,
        2 => Icon::Activity,
        4 => Icon::Blocks,
        _ => Icon::Plug,
    }
}
fn nav_row(ui: &mut egui::Ui, icon: Icon, label: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), theme::BUTTON_PRIMARY),
        egui::Sense::click(),
    );
    let hover = ui.ctx().animate_bool_with_time(
        response.id.with("hover"),
        response.hovered(),
        theme::ANIMATION_FAST,
    );
    if selected || hover > 0.0 {
        let fill = if selected {
            theme::SELECTED
        } else {
            theme::SURFACE
        };
        ui.painter().rect_filled(
            rect,
            4.0,
            fill.gamma_multiply(if selected { 0.65 } else { hover * 0.55 }),
        );
    }
    if selected {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(
                rect.left_top() + egui::vec2(0.0, 10.0),
                egui::vec2(2.0, 16.0),
            ),
            1.0,
            theme::ACCENT,
        );
    }
    let tint = if selected { theme::TEXT } else { MUTED };
    let texture = icons::texture(ui.ctx(), icon);
    ui.painter().image(
        texture.id(),
        egui::Rect::from_min_size(rect.min + egui::vec2(12.0, 10.0), egui::vec2(16.0, 16.0)),
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        tint,
    );
    ui.painter().text(
        rect.min + egui::vec2(40.0, 18.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        tint,
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            label,
        )
    });
    response
}
fn heading_icon(ui: &mut egui::Ui, icon: Icon, text: &str) {
    components::section_heading(ui, icon, text);
}
fn action_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    enabled: bool,
    primary: bool,
) -> egui::Response {
    components::action(ui, icon, label, enabled, primary)
}
fn transport_name(transport: &str) -> &str {
    if transport.starts_with("network:") {
        "Wi-Fi · conexão direta"
    } else if transport.starts_with("rsd:") {
        "Wi-Fi · túnel local"
    } else if transport == "usb" {
        "USB"
    } else {
        transport
    }
}

impl Studio {
    pub(super) fn render_shell(&mut self, ctx: &egui::Context) {
        let narrow = ctx.content_rect().width() < 900.0;
        let sidebar_id = egui::Id::new("tetherdeck-sidebar-open");
        let mut sidebar_open = ctx
            .data(|data| data.get_temp::<bool>(sidebar_id))
            .unwrap_or(!narrow);
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::CTRL, egui::Key::B)) {
            sidebar_open = !sidebar_open;
        }
        if self.busy {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        let connected = self.api_confirmed && !self.selected.is_empty();
        let device_name = self
            .report
            .as_ref()
            .and_then(|r| r.devices.iter().find(|d| d.device_id == self.selected))
            .and_then(|d| d.name.clone())
            .unwrap_or_else(|| "iPhone".into());
        let transport = self
            .report
            .as_ref()
            .and_then(|r| r.devices.iter().find(|d| d.device_id == self.selected))
            .map(|d| transport_name(&d.transport).to_owned())
            .unwrap_or_default();
        egui::TopBottomPanel::top("tetherdeck-titlebar")
            .exact_height(48.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::CHROME)
                    .inner_margin(egui::Margin::symmetric(12, 8)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add(
                        egui::Image::new(&self.brand_texture)
                            .fit_to_exact_size(egui::vec2(24.0, 24.0)),
                    );
                    ui.label(RichText::new("TetherDeck").size(14.0).strong());
                    ui.add_space(8.0);
                    if icons::button(
                        ui,
                        if sidebar_open {
                            Icon::PanelClose
                        } else {
                            Icon::PanelOpen
                        },
                        "Alternar barra lateral · Ctrl+B",
                        false,
                        30.0,
                    )
                    .clicked()
                    {
                        sidebar_open = !sidebar_open;
                    }
                    if !narrow {
                        ui.separator();
                        ui.label(
                            RichText::new(if connected {
                                &device_name
                            } else {
                                "Device workspace"
                            })
                            .color(MUTED),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if action_button(ui, Icon::Monitor, "Abrir tela", connected, false)
                            .clicked()
                        {
                            self.open_remote_window(ctx);
                        }
                        if self.busy {
                            ui.spinner();
                        } else {
                            ui.label(
                                RichText::new(if connected {
                                    "Conectado"
                                } else {
                                    "Sem sessão"
                                })
                                .size(11.5)
                                .color(if connected {
                                    theme::SUCCESS
                                } else {
                                    MUTED
                                }),
                            );
                        }
                    });
                });
            });
        egui::TopBottomPanel::bottom("tetherdeck-status")
            .exact_height(26.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::CHROME)
                    .inner_margin(egui::Margin::symmetric(10, 3)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::image_and_text(
                                icons::image(ui.ctx(), Icon::Activity, 13.0, MUTED),
                                RichText::new(format!("Atividade · {}", self.events.len()))
                                    .size(11.0),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        self.activity_open = !self.activity_open;
                    }
                    ui.separator();
                    let show_version = ui.available_width() >= 500.0;
                    let message_width =
                        (ui.available_width() - if show_version { 145.0 } else { 0.0 }).max(1.0);
                    let message = if self.busy {
                        "Operação em andamento…"
                    } else {
                        self.events.front().map(String::as_str).unwrap_or("Pronto")
                    };
                    ui.add_sized(
                        [message_width, 18.0],
                        egui::Label::new(RichText::new(message).size(11.0).color(MUTED)).truncate(),
                    )
                    .on_hover_text(message);
                    if show_version {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new("Windows · 0.1 Preview")
                                    .size(10.5)
                                    .color(MUTED),
                            );
                        });
                    }
                });
            });
        egui::TopBottomPanel::bottom("tetherdeck-activity")
            .resizable(true)
            .default_height(140.0)
            .height_range(85.0..=260.0)
            .frame(egui::Frame::new().fill(theme::SIDEBAR).inner_margin(12))
            .show_animated(ctx, self.activity_open, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("ATIVIDADE");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::button(ui, Icon::Close, "Fechar painel de atividade", false, 24.0)
                            .clicked()
                        {
                            self.activity_open = false;
                        }
                    });
                });
                egui::ScrollArea::vertical()
                    .id_salt("activity-log")
                    .show(ui, |ui| {
                        for item in &self.events {
                            ui.label(RichText::new(item).size(12.0).color(MUTED));
                        }
                    });
            });
        if let Some(error) = self.error.clone() {
            egui::TopBottomPanel::bottom("tetherdeck-error")
                .frame(
                    egui::Frame::new()
                        .fill(Color32::from_rgb(54, 35, 40))
                        .inner_margin(egui::Margin::symmetric(14, 8)),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(icons::image(ui.ctx(), Icon::Alert, 16.0, theme::ERROR));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icons::button(ui, Icon::Close, "Dispensar aviso", false, 24.0)
                                .clicked()
                            {
                                self.error = None;
                            }
                            ui.add(
                                egui::Label::new(RichText::new(error).color(theme::ERROR)).wrap(),
                            );
                        });
                    });
                });
        }
        egui::SidePanel::left("tetherdeck-activity-rail")
            .resizable(false)
            .exact_width(52.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::CHROME)
                    .inner_margin(egui::Margin::symmetric(5, 8)),
            )
            .show(ctx, |ui| {
                for page in [3, 0, 1, 2, 4] {
                    let selected = self.page == page;
                    let response =
                        icons::button(ui, page_icon(page), page_name(page), selected, 42.0);
                    if selected {
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                response.rect.left_top() + egui::vec2(-5.0, 10.0),
                                egui::vec2(2.0, 22.0),
                            ),
                            1.0,
                            theme::ACCENT,
                        );
                    }
                    if response.clicked() {
                        if selected {
                            sidebar_open = !sidebar_open;
                        } else {
                            self.page = page;
                        }
                    }
                    ui.add_space(4.0);
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    if icons::button(ui, Icon::Help, "Guia e integração com IA", false, 42.0)
                        .clicked()
                    {
                        self.page = 4;
                    }
                });
            });
        egui::SidePanel::left("tetherdeck-sidebar")
            .resizable(true)
            .default_width(222.0)
            .width_range(190.0..=340.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::SIDEBAR)
                    .inner_margin(egui::Margin::symmetric(12, 16)),
            )
            .show_animated(ctx, sidebar_open, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("sidebar-sections")
                    .max_height((ui.available_height() - theme::SPACE_24).max(80.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("WORKSPACE").size(10.0).strong().color(MUTED));
                        });
                        ui.add_space(12.0);
                        for page in [3, 0, 1] {
                            if nav_row(ui, page_icon(page), page_name(page), self.page == page)
                                .clicked()
                            {
                                self.page = page;
                            }
                        }
                        components::gap(ui, theme::SPACE_24);
                        ui.label(
                            RichText::new("FERRAMENTAS")
                                .size(10.0)
                                .strong()
                                .color(MUTED),
                        );
                        ui.add_space(8.0);
                        for page in [2, 4] {
                            if nav_row(ui, page_icon(page), page_name(page), self.page == page)
                                .clicked()
                            {
                                self.page = page;
                            }
                        }
                        ui.add_space(24.0);
                        ui.separator();
                        ui.add_space(12.0);
                        ui.label(RichText::new("SESSÃO").size(10.0).strong().color(MUTED));
                        components::gap(ui, theme::SPACE_8);
                        if connected {
                            heading_icon(
                                ui,
                                if transport.starts_with("Wi-Fi") {
                                    Icon::Wifi
                                } else {
                                    Icon::Usb
                                },
                                &device_name,
                            );
                            ui.label(RichText::new(&transport).size(11.5).color(MUTED));
                            ui.add_space(8.0);
                            if action_button(ui, Icon::Monitor, "Abrir tela", true, false).clicked()
                            {
                                self.open_remote_window(ctx);
                            }
                            if self.local_connection.is_some()
                                && action_button(ui, Icon::Unplug, "Desconectar", !self.busy, false)
                                    .clicked()
                            {
                                self.disconnect_workspace(ctx);
                            }
                        } else {
                            ui.label(RichText::new("Nenhum aparelho ativo").color(MUTED));
                            ui.small("Escolha USB ou Wi-Fi para iniciar uma sessão.");
                        }
                    });
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.label(RichText::new("Open source · MIT").size(10.5).color(MUTED));
                });
            });
        ctx.data_mut(|data| data.insert_temp(sidebar_id, sidebar_open));
        egui::TopBottomPanel::top("tetherdeck-workbench-tabs")
            .exact_height(40.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::SIDEBAR)
                    .inner_margin(egui::Margin::symmetric(16, 8)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add(icons::image(
                        ui.ctx(),
                        page_icon(self.page),
                        15.0,
                        theme::ACCENT,
                    ));
                    ui.label(RichText::new(page_name(self.page)).strong().size(12.5));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if connected {
                            ui.label(RichText::new(&transport).size(11.0).color(MUTED));
                        }
                    });
                });
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BACKGROUND)
                    .inner_margin(if narrow {
                        theme::SPACE_16 as i8
                    } else {
                        theme::SPACE_32 as i8
                    }),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("workspace-page", self.page))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        components::body(ui, |ui| match self.page {
                            3 => self.render_connect(ui, ctx),
                            4 => self.render_integrations(ui, ctx),
                            0 => {
                                self.render_device_picker(ui);
                                self.render_elements(ui, ctx);
                            }
                            _ => {
                                if self.page == 2 {
                                    components::page_header(
                                        ui,
                                        "DIAGNÓSTICO",
                                        "Saúde da conexão",
                                        "Verifique cada camada e escolha a recuperação adequada.",
                                    );
                                    self.render_diagnostic_actions(ui, ctx);
                                } else {
                                    self.render_device_picker(ui);
                                }
                                self.render_workbench(ui, ctx);
                            }
                        })
                    });
            });
    }
    fn disconnect_workspace(&mut self, ctx: &egui::Context) {
        self.close_remote_window(ctx);
        self.live_feed = None;
        self.control_enabled = false;
        self.remote_drag = None;
        self.texture = None;
        self.snapshot_at = None;
        self.local_connection.take();
        self.token.clear();
        self.diagnose_local(ctx);
        self.page = 3;
    }
    fn render_device_picker(&mut self, ui: &mut egui::Ui) {
        let devices = self
            .report
            .as_ref()
            .map(|r| r.devices.clone())
            .unwrap_or_default();
        if self.api_confirmed && !devices.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.small("APARELHO");
                for device in devices {
                    if ui
                        .add_enabled(
                            !self.busy,
                            egui::Button::selectable(
                                self.selected == device.device_id,
                                device.name.as_deref().unwrap_or(&device.device_id),
                            ),
                        )
                        .clicked()
                    {
                        self.selected = device.device_id;
                        self.elements = None;
                        self.texture = None;
                        self.snapshot_at = None;
                        self.control_enabled = false;
                    }
                }
            });
            ui.add_space(16.0);
        } else {
            card(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Conecte um aparelho para começar.");
                    if ui.button("Ir para Conectar").clicked() {
                        self.page = 3;
                    }
                });
            });
            ui.add_space(16.0);
        }
    }
    fn render_connect(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.local_connection.is_some() {
            title(
                ui,
                "SESSÃO ATIVA",
                "Seu aparelho está conectado.",
                "Abra a tela ao vivo ou continue nas ferramentas do workspace.",
            );
            let name = self
                .report
                .as_ref()
                .and_then(|report| {
                    report
                        .devices
                        .iter()
                        .find(|device| device.device_id == self.selected)
                })
                .and_then(|device| device.name.clone())
                .unwrap_or_else(|| "iPhone".into());
            let mode = self
                .local_connection
                .as_ref()
                .map(|connection| connection.mode)
                .unwrap_or(0);
            card(ui, |ui| {
                heading_icon(ui, if mode == 0 { Icon::Usb } else { Icon::Wifi }, &name);
                ui.label(
                    RichText::new(match mode {
                        0 => "USB · conexão local",
                        1 => "Wi-Fi · conexão direta sem cabo",
                        _ => "Wi-Fi · túnel local",
                    })
                    .size(theme::TEXT_LABEL)
                    .color(theme::SUCCESS),
                );
                components::gap(ui, theme::SPACE_16);
                ui.horizontal_wrapped(|ui| {
                    if action_button(ui, Icon::Monitor, "Abrir tela ao vivo", true, true).clicked()
                    {
                        self.open_remote_window(ctx);
                    }
                    if action_button(ui, Icon::Unplug, "Desconectar", !self.busy, false).clicked() {
                        self.disconnect_workspace(ctx);
                    }
                });
            });
            components::gap(ui, theme::GAP);
            if ui.available_width() >= 780.0 {
                components::two_columns(ui, |columns| {
                    self.render_connected_elements(&mut columns[0], mode);
                    self.render_connected_diagnostics(&mut columns[1]);
                });
            } else {
                self.render_connected_elements(ui, mode);
                components::gap(ui, theme::GAP);
                self.render_connected_diagnostics(ui);
            }
            return;
        }
        title(
            ui,
            "CONECTAR APARELHO",
            "Conecte seu iPhone.",
            "USB para começar. Wi-Fi para trabalhar sem cabo depois do pareamento inicial.",
        );
        ui.horizontal_wrapped(|ui| {
            let editable = !self.busy && self.local_connection.is_none();
            for (mode, icon, label) in [(0, Icon::Usb, "USB"), (1, Icon::Wifi, "Wi-Fi")] {
                let selected = if mode == 0 {
                    self.connection_mode == 0
                } else {
                    self.connection_mode != 0
                };
                let button = egui::Button::image_and_text(
                    icons::image(
                        ui.ctx(),
                        icon,
                        17.0,
                        if selected { theme::ACCENT } else { MUTED },
                    ),
                    label,
                )
                .selected(selected)
                .min_size(egui::vec2(112.0, 36.0));
                if ui.add_enabled(editable, button).clicked() && self.connection_mode != mode {
                    self.choose_connection_mode(mode);
                }
            }
            if self.busy {
                ui.spinner();
                ui.label(RichText::new("Verificando conexão…").color(MUTED));
            }
        });
        components::gap(ui, theme::SPACE_24);
        if ui.available_width() >= 780.0 {
            components::two_columns(ui, |columns| {
                self.render_transport_setup(&mut columns[0], ctx);
                self.render_session_options(&mut columns[1]);
            });
            components::gap(ui, theme::GAP);
            components::two_columns(ui, |columns| {
                self.render_connection_devices(&mut columns[0], ctx);
                self.render_connection_help(&mut columns[1], ctx);
            });
        } else {
            self.render_transport_setup(ui, ctx);
            components::gap(ui, theme::SPACE_16);
            self.render_connection_devices(ui, ctx);
            components::gap(ui, theme::SPACE_16);
            self.render_session_options(ui);
            components::gap(ui, theme::SPACE_16);
            self.render_connection_help(ui, ctx);
        }
    }
    fn render_connected_elements(&mut self, ui: &mut egui::Ui, mode: u8) {
        card(ui, |ui| {
            heading_icon(ui, Icon::Scan, "Automação por elementos");
            ui.label(
                "Inspecione controles e encontre identificadores para seus fluxos de automação.",
            );
            components::gap(ui, theme::SPACE_16);
            if action_button(ui, Icon::Scan, "Abrir elementos", mode != 1, false)
                .on_hover_text(if mode == 1 {
                    "Elementos WDA não estão disponíveis nesta conexão Wi-Fi direta."
                } else {
                    "Inspecionar a tela atual do aparelho"
                })
                .clicked()
            {
                self.page = 0;
            }
        });
    }
    fn render_connected_diagnostics(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            heading_icon(ui, Icon::Activity, "Saúde da conexão");
            ui.label(
                "Consulte transporte, serviços e automação antes de escolher uma recuperação.",
            );
            components::gap(ui, theme::SPACE_16);
            if action_button(ui, Icon::Activity, "Abrir diagnóstico", true, false).clicked() {
                self.page = 2;
            }
        });
    }
    fn choose_connection_mode(&mut self, mode: u8) {
        if mode == 1 && self.wifi_udid.is_empty() {
            if let Some(report) = &self.report {
                let ids: Vec<_> = report
                    .devices
                    .iter()
                    .filter(|d| d.transport == "usb")
                    .filter_map(|d| d.udid.clone())
                    .collect();
                if ids.len() == 1 {
                    self.wifi_udid = ids[0].clone();
                } else if report.usb_recovery_targets.len() == 1 {
                    self.wifi_udid = report.usb_recovery_targets[0].device_id.clone();
                }
            }
        }
        self.connection_mode = mode;
        self.report = None;
        self.api_confirmed = false;
        self.selected.clear();
        self.elements = None;
        self.selected_element.clear();
        self.texture = None;
        self.snapshot_at = None;
        self.control_enabled = false;
        self.runner_configured = false;
        self.error = None;
    }
    fn connection_has_ready_device(&self) -> bool {
        self.report.as_ref().is_some_and(|report| {
            report.devices.iter().any(|device| {
                let matching_transport = match self.connection_mode {
                    0 => device.transport == "usb",
                    1 => device.transport.starts_with("network:"),
                    _ => device.transport.starts_with("rsd:"),
                };
                matching_transport
                    && matches!(
                        device.identity_status,
                        rust_ios_device_api::IdentityStatus::Available
                    )
            })
        })
    }
    fn render_transport_setup(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let has_ready_device = self.connection_has_ready_device();
        card(ui, |ui| {
            heading_icon(
                ui,
                if self.connection_mode == 0 {
                    Icon::Usb
                } else {
                    Icon::Wifi
                },
                if self.connection_mode == 0 {
                    "Conexão USB"
                } else {
                    "Conexão Wi-Fi"
                },
            );
            if self.connection_mode == 0 {
                ui.label("Conecte o iPhone desbloqueado com um cabo de dados e aceite a solicitação de confiança, se aparecer.");
                ui.add_space(12.0);
                if action_button(
                    ui,
                    Icon::Refresh,
                    "Buscar aparelhos USB",
                    !self.busy,
                    !has_ready_device,
                )
                .clicked()
                {
                    self.diagnose_local(ctx);
                }
                components::gap(ui, theme::SPACE_8);
                ui.small("O suporte a dispositivos Apple e seu serviço precisam estar instalados no Windows.");
            } else {
                ui.label("Mantenha o iPhone desbloqueado e na mesma rede do computador. O aparelho precisa estar pareado com este Windows.");
                components::gap(ui, theme::SPACE_12);
                ui.label(
                    RichText::new("Identificador do aparelho (UDID)")
                        .size(12.0)
                        .strong(),
                );
                let changed = ui
                    .add_enabled(
                        !self.busy,
                        egui::TextEdit::singleline(&mut self.wifi_udid)
                            .hint_text("UDID do iPhone já pareado")
                            .desired_width(f32::INFINITY),
                    )
                    .changed();
                if changed {
                    self.report = None;
                }
                components::gap(ui, theme::SPACE_8);
                if action_button(
                    ui,
                    Icon::Refresh,
                    "Buscar no Wi-Fi",
                    !self.busy && !self.wifi_udid.trim().is_empty(),
                    !has_ready_device,
                )
                .clicked()
                {
                    self.connection_mode = 1;
                    self.diagnose_local(ctx);
                }
                ui.add_space(8.0);
                ui.small("A busca usa a rede local. Esta sessão não muda para USB.");
                components::gap(ui, theme::SPACE_8);
                ui.collapsing("IP manual", |ui| {
                    ui.small("Use apenas se a descoberta automática não localizar o aparelho.");
                    if ui
                        .add_enabled(
                            !self.busy,
                            egui::TextEdit::singleline(&mut self.wifi_host)
                                .hint_text("IP local do iPhone")
                                .desired_width(f32::INFINITY),
                        )
                        .changed()
                    {
                        self.report = None;
                    }
                });
            }
        });
    }
    fn render_connection_devices(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        card(ui, |ui| {
            heading_icon(ui, Icon::Plug, "Aparelhos encontrados");
            let devices: Vec<_> = self
                .report
                .as_ref()
                .map(|r| r.devices.clone())
                .unwrap_or_default()
                .into_iter()
                .filter(|d| match self.connection_mode {
                    0 => d.transport == "usb",
                    1 => d.transport.starts_with("network:"),
                    _ => d.transport.starts_with("rsd:"),
                })
                .collect();
            if devices.is_empty() {
                components::empty_state(
                    ui,
                    Icon::Plug,
                    if self.busy {
                        "Procurando seu iPhone"
                    } else if self.report.is_some() {
                        "Nenhum aparelho disponível"
                    } else {
                        "Pronto para buscar"
                    },
                    if self.busy {
                        "Aguarde a verificação da conexão e da identidade do aparelho."
                    } else {
                        "Desbloqueie o iPhone e faça uma busca no transporte selecionado."
                    },
                );
                if self.report.is_some() {
                    components::gap(ui, theme::SPACE_16);
                    if action_button(
                        ui,
                        Icon::Activity,
                        "Ver diagnóstico da busca",
                        !self.busy,
                        false,
                    )
                    .clicked()
                    {
                        self.page = 2;
                    }
                }
            }
            let mut primary_available = true;
            for device in devices {
                ui.strong(device.name.as_deref().unwrap_or("iPhone"));
                ui.label(
                    RichText::new(transport_name(&device.transport))
                        .size(12.0)
                        .color(MUTED),
                );
                let ready = matches!(
                    device.identity_status,
                    rust_ios_device_api::IdentityStatus::Available
                );
                ui.label(
                    RichText::new(if ready {
                        "Identidade verificada"
                    } else {
                        "Conexão ainda não disponível"
                    })
                    .size(11.5)
                    .color(if ready {
                        theme::SUCCESS
                    } else {
                        theme::WARNING
                    }),
                );
                ui.add_space(8.0);
                let primary = ready && primary_available;
                if ready {
                    primary_available = false;
                }
                if action_button(ui, Icon::Plug, "Abrir sessão", !self.busy && ready, primary)
                    .clicked()
                {
                    self.connect_local(
                        device.udid.clone().unwrap_or(device.device_id.clone()),
                        ctx,
                    );
                }
                ui.collapsing("Identificação técnica", |ui| {
                    ui.label(RichText::new(&device.device_id).monospace().size(11.0));
                });
                components::gap(ui, theme::SPACE_8);
            }
        });
    }
    fn render_session_options(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            heading_icon(ui, Icon::Settings, "Opções da sessão");
            ui.label("A tela e os controles por gesto ficam na janela do aparelho. Automação por elementos usa um runner WDA próprio.");
            components::gap(ui, theme::SPACE_12);
            ui.collapsing("Automação por elementos", |ui| {
                ui.label("Bundle ID do runner instalado");
                ui.add_enabled(
                    !self.busy,
                    egui::TextEdit::singleline(&mut self.runner_bundle)
                        .desired_width(f32::INFINITY),
                );
                ui.small(
                    "O runner deve estar assinado para este iPhone pela sua conta ou equipe Apple.",
                );
                if self.connection_mode == 1 {
                    ui.colored_label(
                        theme::WARNING,
                        "WDA ainda não está disponível nesta conexão Wi-Fi direta.",
                    );
                }
            });
            components::gap(ui, theme::SPACE_8);
            ui.collapsing("Recuperação de conexão",|ui|{
                ui.checkbox(&mut self.local_allow_usb_recovery,"Permitir recuperação da interface USB");
                ui.small("Exige administrador. A recuperação é solicitada manualmente no diagnóstico e não reinicia o computador.");
            });
            components::gap(ui, theme::SPACE_12);
            if ui.link("Assinatura e primeiros passos").clicked() {
                self.page = 4;
            }
        });
    }
    fn render_connection_help(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        card(ui, |ui| {
            heading_icon(
                ui,
                Icon::Help,
                if self.connection_mode == 0 {
                    "Primeira conexão"
                } else {
                    "Trabalhar sem cabo"
                },
            );
            if self.connection_mode == 0 {
                ui.label("1. Conecte e desbloqueie o iPhone.");
                ui.label("2. Confirme a confiança no computador.");
                ui.label("3. Busque o aparelho e abra a sessão.");
                ui.add_space(8.0);
                ui.small("Depois do primeiro pareamento, você pode preparar a conexão Wi-Fi neste mesmo computador.");
            } else {
                ui.label("1. Faça o pareamento inicial por USB.");
                ui.label("2. Habilite a conexão Wi-Fi do iPhone.");
                ui.label("3. Retire o cabo e busque pela rede.");
                ui.add_space(8.0);
                ui.collapsing("Habilitar Wi-Fi pelo USB pareado",|ui|{
                    ui.small("Conecte o iPhone por USB nesta etapa. Isso habilita a conexão futura pela rede sem criar um novo pareamento.");
                    if action_button(ui,Icon::Wifi,"Habilitar conexão Wi-Fi",!self.busy&&!self.wifi_udid.trim().is_empty(),false).clicked(){self.enable_wifi(ctx);}
                });
                ui.collapsing("Túnel local RSD · avançado",|ui|{
                    ui.small("Use um túnel já estabelecido neste computador. Ele oferece tela, gestos e abertura de apps; não oferece elementos WDA sozinho.");
                    ui.label("Serviço de túneis local");
                    if ui.add_enabled(!self.busy,egui::TextEdit::singleline(&mut self.tunneld_origin).desired_width(f32::INFINITY)).changed(){self.report=None;}
                    if action_button(ui,Icon::Refresh,"Verificar túnel",!self.busy&&!self.wifi_udid.trim().is_empty(),false).clicked(){self.connection_mode=2;self.diagnose_local(ctx);}
                    let confirmed=self.report_label=="Computador local"&&self.connection_mode==2&&self.report.as_ref().is_some_and(|r|r.devices.iter().any(|d|d.transport.starts_with("rsd:")&&d.udid.as_deref()==Some(self.wifi_udid.as_str())));
                    if action_button(ui,Icon::Plug,"Abrir sessão pelo túnel",!self.busy&&confirmed,false).clicked(){self.connect_local(self.wifi_udid.clone(),ctx);}
                    ui.hyperlink_to("Guia do serviço de túneis","https://github.com/doronz88/pymobiledevice3/blob/master/docs/guides/ios17-tunnels.md");
                });
                ui.add_space(8.0);
                ui.hyperlink_to(
                    "Wi-Fi no Windows · guia Apple",
                    "https://support.apple.com/guide/itunes/itns3751d862/windows",
                );
            }
        });
    }
    fn render_diagnostic_actions(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal_wrapped(|ui|{
            if ui.add_enabled(!self.busy,egui::Button::new("Atualizar diagnóstico")).clicked(){if self.api_confirmed{self.dispatch(Action::Diagnose,ctx);}else{self.diagnose_local(ctx);}}
            ui.collapsing("Recuperação",|ui|{
                ui.label("Reiniciar o serviço afeta as conexões Apple deste computador. Exige privilégios e não repete ações no aparelho.");
                let local=self.local_recovery_available();let cooldown=self.local_recovery_remaining();
                let enabled=self.api_confirmed&&self.report.as_ref().is_some_and(|r|r.recovery_enabled);
                if ui.add_enabled(!self.busy && if local{cooldown==0}else{enabled},egui::Button::new("Reiniciar serviço Apple/usbmux")).clicked(){if local{self.recover_local(local::LocalRecovery::Service,ctx);}else{self.dispatch(Action::Restart,ctx);}}
                if cooldown>0{ui.small(format!("Nova recuperação local em {cooldown} s."));ctx.request_repaint_after(Duration::from_secs(1));}
                if ui.add_enabled(!self.busy&&self.runner_configured&&self.api_confirmed,egui::Button::new("Reiniciar automação WDA")).clicked(){self.dispatch(Action::RestartElements,ctx);}
            });
        });
        components::gap(ui, theme::SPACE_24);
    }
    fn render_integrations(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        title(
            ui,
            "INTEGRAÇÕES",
            "Conecte suas ferramentas.",
            "Use sua IA, configure uma sessão existente ou prepare a automação por elementos.",
        );
        if ui.available_width() >= 780.0 {
            components::two_columns(ui, |columns| {
                self.render_mcp_integration(&mut columns[0], ctx);
                self.render_signing_guide(&mut columns[1]);
            });
            components::gap(ui, theme::GAP);
            components::two_columns(ui, |columns| {
                self.render_first_steps(&mut columns[0]);
                self.render_existing_server(&mut columns[1], ctx);
            });
        } else {
            self.render_mcp_integration(ui, ctx);
            components::gap(ui, theme::GAP);
            self.render_signing_guide(ui);
            components::gap(ui, theme::GAP);
            self.render_first_steps(ui);
            components::gap(ui, theme::GAP);
            self.render_existing_server(ui, ctx);
        }
    }
    fn render_mcp_integration(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        card(ui, |ui| {
            heading_icon(ui, Icon::Blocks, "Sua IA por MCP");
            ui.label(
                "Consulte a tela, encontre um elemento e confirme o resultado após cada ação.",
            );
            components::gap(ui, theme::SPACE_8);
            egui::Frame::new()
                .fill(theme::CHROME)
                .corner_radius(theme::RADIUS_BUTTON)
                .inner_margin(12)
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(
                        RichText::new(
                            "ui_snapshot → find_element\n→ activate_element → ui_snapshot",
                        )
                        .monospace()
                        .size(theme::TEXT_LABEL)
                        .color(theme::MUTED),
                    );
                });
            components::gap(ui, theme::SPACE_16);
            if action_button(
                ui,
                Icon::Blocks,
                "Copiar configuração MCP desta sessão",
                self.local_connection.is_some() && !self.busy,
                true,
            )
            .clicked()
            {
                self.copy_mcp_config(ctx);
                self.log("Configuração MCP copiada por solicitação. Ela contém a credencial da sessão atual.".into());
            }
            components::gap(ui, theme::SPACE_8);
            ui.label(RichText::new("A configuração inclui o token temporário. Cole somente no cliente de IA em que você confia e copie novamente ao abrir outra sessão.").size(theme::TEXT_LABEL).color(MUTED));
        });
    }
    fn render_signing_guide(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            heading_icon(ui, Icon::Lock, "Assinatura do iOS");
            ui.label("Cada usuário ou equipe prepara seu próprio WebDriverAgent assinado para seus aparelhos.");
            components::gap(ui, theme::SPACE_8);
            ui.label(RichText::new("O aplicativo Windows não usa um certificado Apple. O runner no iPhone precisa de certificado, perfil e aparelho autorizados pela sua equipe.").color(MUTED));
            components::gap(ui, theme::SPACE_8);
            ui.hyperlink_to("Preparar runner · Appium", "https://github.com/appium/appium-xcuitest-driver/blob/master/docs/getting-started/provisioning-profile/generic-device-config.md");
            ui.hyperlink_to("Perfis de desenvolvimento · Apple", "https://developer.apple.com/help/account/provisioning-profiles/create-a-development-provisioning-profile");
            components::gap(ui, theme::SPACE_8);
            ui.label(RichText::new("O pacote da comunidade não inclui chave privada nem runner pessoal. Um IPA de outra pessoa não autoriza automaticamente o seu aparelho.").size(theme::TEXT_LABEL).color(MUTED));
        });
    }
    fn render_first_steps(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            heading_icon(ui, Icon::Plug, "Primeiros passos");
            ui.label("1. Extraia o pacote e mantenha os executáveis juntos.");
            ui.label("2. Em Conectar, escolha USB ou Wi-Fi e busque o aparelho.");
            ui.label("3. Abra a sessão e use Abrir tela para visualizar o iPhone.");
            components::gap(ui, theme::SPACE_8);
            ui.label(RichText::new("Para elementos, prepare seu runner WDA antes. Gratuito e open source · MIT · Windows.").size(theme::TEXT_LABEL).color(MUTED));
        });
    }
    fn render_existing_server(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        card(ui, |ui| {
            heading_icon(ui, Icon::Settings, "Servidor existente");
            ui.label(RichText::new("Use um daemon local ou um encaminhamento SSH. Os dados da conexão ficam em memória.").color(MUTED));
            components::gap(ui, theme::SPACE_8);
            ui.collapsing("Configurar endereço e token", |ui| {
                let editable = !self.busy && self.local_connection.is_none();
                ui.label(RichText::new("Endereço da API").size(theme::TEXT_LABEL));
                let changed = ui
                    .add_enabled(
                        editable,
                        egui::TextEdit::singleline(&mut self.origin).desired_width(f32::INFINITY),
                    )
                    .changed();
                ui.label(RichText::new("Token da sessão").size(theme::TEXT_LABEL));
                let changed_token = ui
                    .add_enabled(
                        editable,
                        egui::TextEdit::singleline(&mut self.token)
                            .password(true)
                            .desired_width(f32::INFINITY),
                    )
                    .changed();
                if changed || changed_token {
                    self.api_confirmed = false;
                    self.report = None;
                    self.selected.clear();
                    self.elements = None;
                    self.texture = None;
                    self.snapshot_at = None;
                    self.control_enabled = false;
                    self.runner_configured = false;
                    self.usb_recovery_enabled = false;
                }
                components::gap(ui, theme::SPACE_8);
                if action_button(
                    ui,
                    Icon::Plug,
                    "Conectar ao servidor",
                    editable && !self.token.trim().is_empty(),
                    false,
                )
                .clicked()
                {
                    self.dispatch(Action::Diagnose, ctx);
                }
            });
        });
    }
    fn enable_wifi(&mut self, ctx: &egui::Context) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        let id = self.wifi_udid.trim().to_owned();
        let (sender, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        let context = ctx.clone();
        self.runtime.spawn(async move{
            let result=local::local_command(&["--device",&id,"--enable-wifi"],20,false).await;
            let event=match result{
                Ok(bytes) if serde_json::from_slice::<serde_json::Value>(&bytes).ok().is_some_and(|v|v["code"]=="wifi_setting_enabled")=>Event::Message("Conexões Wi-Fi habilitadas no iPhone. Agora busque aparelhos na mesma rede; a descoberta ainda não foi confirmada.".into()),
                _=>Event::Failed("Não foi possível confirmar a preparação Wi-Fi. Verifique a conexão USB confiável e consulte o diagnóstico antes de repetir; o resultado da alteração pode ser incerto.".into()),
            };let _=sender.send(event);context.request_repaint();
        });
    }
}
