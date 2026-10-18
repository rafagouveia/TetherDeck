//! Shared composition primitives keep spacing and control states consistent across tools.
use super::{
    icons::{self, Icon},
    theme,
};
use eframe::egui::{self, RichText};

/// Tokens describe the visible gap, including egui's automatic item spacing.
pub(super) fn gap(ui: &mut egui::Ui, size: f32) {
    let automatic = if ui.layout().is_horizontal() {
        ui.spacing().item_spacing.x
    } else {
        ui.spacing().item_spacing.y
    };
    ui.add_space(size - automatic);
}

pub(super) fn card<R>(
    ui: &mut egui::Ui,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    egui::Frame::new()
        .fill(theme::SURFACE)
        .stroke(egui::Stroke::new(1.0, theme::BORDER))
        .corner_radius(theme::RADIUS_CARD)
        .inner_margin(theme::CARD_PADDING)
        .show(ui, |ui| {
            let width = ui.available_width();
            ui.set_min_width(width);
            // Columns justify their children; card text should retain natural word spacing.
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_min_width(width);
                ui.spacing_mut().item_spacing = egui::vec2(theme::SPACE_8, theme::SPACE_8);
                add(ui)
            })
            .inner
        })
}

pub(super) fn body<R>(
    ui: &mut egui::Ui,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let available = ui.available_rect_before_wrap();
    let width = available.width().min(theme::CONTENT_MAX);
    let inset = ((available.width() - width) * 0.5).max(0.0);
    let rect = egui::Rect::from_min_max(
        available.min + egui::vec2(inset, 0.0),
        egui::pos2(available.min.x + inset + width, available.max.y),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.set_width(width);
            add(ui)
        },
    )
}

pub(super) fn two_columns<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut [egui::Ui]) -> R) -> R {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = theme::GAP;
        ui.columns(2, add)
    })
    .inner
}

pub(super) fn page_header(ui: &mut egui::Ui, eyebrow: &str, title: &str, description: &str) {
    ui.label(
        RichText::new(eyebrow)
            .size(10.0)
            .color(theme::MUTED)
            .strong(),
    );
    gap(ui, theme::SPACE_4);
    ui.label(
        RichText::new(title)
            .size(theme::TEXT_TITLE)
            .strong()
            .color(theme::TEXT),
    );
    if !description.is_empty() {
        ui.label(
            RichText::new(description)
                .size(theme::TEXT_BODY)
                .color(theme::MUTED),
        );
    }
    gap(ui, theme::SPACE_24);
}

pub(super) fn section_heading(ui: &mut egui::Ui, icon: Icon, label: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SPACE_8;
        ui.add(icons::image(ui.ctx(), icon, 18.0, theme::ACCENT));
        ui.label(RichText::new(label).size(theme::TEXT_BODY).strong());
    });
    gap(ui, theme::SPACE_8);
}

pub(super) fn empty_state(ui: &mut egui::Ui, icon: Icon, title: &str, description: &str) {
    ui.horizontal_top(|ui| {
        egui::Frame::new()
            .fill(theme::SELECTED)
            .corner_radius(theme::RADIUS_CARD)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.add(icons::image(ui.ctx(), icon, 20.0, theme::ACCENT));
            });
        gap(ui, theme::SPACE_4);
        ui.vertical(|ui| {
            ui.label(RichText::new(title).size(theme::TEXT_BODY).strong());
            ui.label(
                RichText::new(description)
                    .size(theme::TEXT_LABEL)
                    .color(theme::MUTED),
            );
        });
    });
}

pub(super) fn action(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    enabled: bool,
    primary: bool,
) -> egui::Response {
    ui.scope(|ui| {
        let height = if primary {
            theme::BUTTON_PRIMARY
        } else {
            theme::BUTTON_SECONDARY
        };
        let tint = if primary {
            theme::PRIMARY_TEXT
        } else {
            theme::TEXT
        };
        ui.spacing_mut().button_padding = egui::vec2(theme::SPACE_12, theme::SPACE_4);
        if primary {
            let widgets = &mut ui.visuals_mut().widgets;
            widgets.inactive.weak_bg_fill = theme::PRIMARY;
            widgets.inactive.bg_fill = theme::PRIMARY;
            widgets.inactive.bg_stroke = egui::Stroke::new(1.0, theme::PRIMARY);
            widgets.hovered.weak_bg_fill = theme::PRIMARY_HOVER;
            widgets.hovered.bg_fill = theme::PRIMARY_HOVER;
            widgets.hovered.bg_stroke = egui::Stroke::new(1.0, theme::ACCENT);
            widgets.active.weak_bg_fill = theme::PRIMARY;
            widgets.active.bg_fill = theme::PRIMARY;
        }
        let button = egui::Button::image_and_text(
            icons::image(ui.ctx(), icon, 16.0, tint),
            RichText::new(label).size(13.0).color(tint),
        )
        .corner_radius(theme::RADIUS_BUTTON)
        .min_size(egui::vec2(0.0, height));
        ui.add_enabled(enabled, button)
    })
    .inner
}
