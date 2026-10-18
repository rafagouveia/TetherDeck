//! Shared visual tokens for TetherDeck's Windows device workspace.
use eframe::egui::{self, Color32, FontId, Stroke};

pub const BACKGROUND: Color32 = Color32::from_rgb(23, 26, 32);
pub const SIDEBAR: Color32 = Color32::from_rgb(27, 31, 39);
pub const CHROME: Color32 = Color32::from_rgb(20, 23, 29);
pub const SURFACE: Color32 = Color32::from_rgb(32, 37, 46);
pub const BORDER: Color32 = Color32::from_rgb(48, 55, 68);
pub const TEXT: Color32 = Color32::from_rgb(231, 236, 244);
pub const MUTED: Color32 = Color32::from_rgb(152, 165, 184);
pub const ACCENT: Color32 = Color32::from_rgb(139, 174, 255);
pub const SELECTED: Color32 = Color32::from_rgb(43, 56, 80);
pub const SUCCESS: Color32 = Color32::from_rgb(132, 204, 172);
pub const WARNING: Color32 = Color32::from_rgb(235, 193, 126);
pub const ERROR: Color32 = Color32::from_rgb(242, 151, 151);
pub const PRIMARY: Color32 = Color32::from_rgb(65, 100, 187);
pub const PRIMARY_HOVER: Color32 = Color32::from_rgb(74, 111, 206);
pub const PRIMARY_TEXT: Color32 = Color32::from_rgb(245, 248, 255);
pub const SPACE_4: f32 = 4.0;
pub const SPACE_8: f32 = 8.0;
pub const SPACE_12: f32 = 12.0;
pub const SPACE_16: f32 = 16.0;
pub const SPACE_24: f32 = 24.0;
pub const SPACE_32: f32 = 32.0;
pub const GAP: f32 = SPACE_16;
pub const CARD_PADDING: i8 = 24;
pub const CONTENT_MAX: f32 = 1120.0;
pub const BUTTON_PRIMARY: f32 = 36.0;
pub const BUTTON_SECONDARY: f32 = 32.0;
pub const RADIUS_CARD: u8 = 8;
pub const RADIUS_BUTTON: u8 = 6;
pub const TEXT_BODY: f32 = 14.0;
pub const TEXT_LABEL: f32 = 12.0;
pub const TEXT_TITLE: f32 = 24.0;
pub const ANIMATION_FAST: f32 = 0.12;
pub const ANIMATION_NORMAL: f32 = 0.16;

pub fn apply(ctx: &egui::Context) {
    // Use the operating system UI font without redistributing a proprietary font.
    #[cfg(target_os = "windows")]
    if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\segoeui.ttf") {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "windows-ui".into(),
            egui::FontData::from_owned(bytes).into(),
        );
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "windows-ui".into());
        ctx.set_fonts(fonts);
    }
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BACKGROUND;
    visuals.window_fill = SURFACE;
    visuals.extreme_bg_color = CHROME;
    visuals.faint_bg_color = SURFACE;
    visuals.code_bg_color = CHROME;
    visuals.override_text_color = Some(TEXT);
    visuals.weak_text_color = Some(MUTED);
    visuals.selection.bg_fill = SELECTED;
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT;
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.window_corner_radius = 8.into();
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, MUTED);
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(38, 43, 53);
    visuals.widgets.inactive.bg_fill = SURFACE;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(48, 55, 68);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(48, 55, 68);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(75, 89, 112));
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.active.weak_bg_fill = SELECTED;
    visuals.widgets.active.bg_fill = SELECTED;
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
    ] {
        widget.corner_radius = RADIUS_BUTTON.into();
    }
    ctx.set_visuals(visuals);
    ctx.style_mut(|style| {
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(TEXT_BODY));
        style
            .text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(13.0));
        style
            .text_styles
            .insert(egui::TextStyle::Small, FontId::proportional(TEXT_LABEL));
        style
            .text_styles
            .insert(egui::TextStyle::Heading, FontId::proportional(TEXT_TITLE));
        style
            .text_styles
            .insert(egui::TextStyle::Monospace, FontId::monospace(12.0));
        style.spacing.item_spacing = egui::vec2(SPACE_8, SPACE_8);
        style.spacing.button_padding = egui::vec2(SPACE_12, SPACE_8);
        style.spacing.interact_size.y = BUTTON_SECONDARY;
        style.spacing.text_edit_width = 240.0;
        style.animation_time = ANIMATION_NORMAL;
        style.wrap_mode = Some(egui::TextWrapMode::Wrap);
    });
}
