//! Lucide ISC icons, rasterized from the retained upstream SVG sources.
use eframe::egui::{self, Color32};

#[derive(Clone, Copy, Debug)]
pub(super) enum Icon {
    Play,
    Pause,
    Minimize,
    MousePointer,
    Maximize,
    Plug,
    Scan,
    Monitor,
    Activity,
    Blocks,
    PanelClose,
    PanelOpen,
    Wifi,
    Usb,
    Close,
    Help,
    Alert,
    Refresh,
    Settings,
    Unplug,
    Home,
    AppWindow,
    VolumeUp,
    VolumeDown,
    Lock,
    Camera,
}
impl Icon {
    fn bytes(self) -> &'static [u8] {
        match self {
            Self::Maximize => include_bytes!("../assets/icons/maximize-2.png"),
            Self::Play => include_bytes!("../assets/icons/play.png"),
            Self::Pause => include_bytes!("../assets/icons/pause.png"),
            Self::Minimize => include_bytes!("../assets/icons/minimize-2.png"),
            Self::MousePointer => include_bytes!("../assets/icons/mouse-pointer-2.png"),
            Self::Home => include_bytes!("../assets/icons/house.png"),
            Self::AppWindow => include_bytes!("../assets/icons/app-window.png"),
            Self::VolumeUp => include_bytes!("../assets/icons/volume-2.png"),
            Self::VolumeDown => include_bytes!("../assets/icons/volume-1.png"),
            Self::Lock => include_bytes!("../assets/icons/lock.png"),
            Self::Camera => include_bytes!("../assets/icons/camera.png"),
            Self::Plug => include_bytes!("../assets/icons/plug.png"),
            Self::Scan => include_bytes!("../assets/icons/scan-line.png"),
            Self::Monitor => include_bytes!("../assets/icons/monitor.png"),
            Self::Activity => include_bytes!("../assets/icons/activity.png"),
            Self::Blocks => include_bytes!("../assets/icons/blocks.png"),
            Self::PanelClose => include_bytes!("../assets/icons/panel-left-close.png"),
            Self::PanelOpen => include_bytes!("../assets/icons/panel-left-open.png"),
            Self::Wifi => include_bytes!("../assets/icons/wifi.png"),
            Self::Usb => include_bytes!("../assets/icons/usb.png"),
            Self::Close => include_bytes!("../assets/icons/x.png"),
            Self::Help => include_bytes!("../assets/icons/circle-help.png"),
            Self::Alert => include_bytes!("../assets/icons/circle-alert.png"),
            Self::Refresh => include_bytes!("../assets/icons/refresh-cw.png"),
            Self::Settings => include_bytes!("../assets/icons/settings-2.png"),
            Self::Unplug => include_bytes!("../assets/icons/unplug.png"),
        }
    }
}
pub(super) fn texture(ctx: &egui::Context, icon: Icon) -> egui::TextureHandle {
    let id = egui::Id::new(format!("lucide-{icon:?}"));
    if let Some(texture) = ctx.data(|data| data.get_temp::<egui::TextureHandle>(id)) {
        return texture;
    }
    let image = image::load_from_memory(icon.bytes())
        .expect("bundled Lucide PNG")
        .into_rgba8();
    let texture = ctx.load_texture(
        format!("lucide-{icon:?}"),
        egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        ),
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
    texture
}
pub(super) fn image(
    ctx: &egui::Context,
    icon: Icon,
    size: f32,
    tint: Color32,
) -> egui::Image<'static> {
    let texture = texture(ctx, icon);
    egui::Image::new((texture.id(), egui::vec2(size, size))).tint(tint)
}
pub(super) fn button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    selected: bool,
    size: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
    let hover =
        ui.ctx()
            .animate_bool_with_time(response.id.with("hover"), response.hovered(), 0.12);
    if selected || hover > 0.0 {
        let alpha = if selected { 255 } else { (hover * 160.0) as u8 };
        let fill = super::theme::SELECTED;
        ui.painter().rect_filled(
            rect.shrink(3.0),
            5.0,
            Color32::from_rgba_unmultiplied(fill.r(), fill.g(), fill.b(), alpha),
        );
    }
    let texture = texture(ui.ctx(), icon);
    let icon_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(20.0, 20.0));
    ui.painter().image(
        texture.id(),
        icon_rect,
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        if selected || response.hovered() {
            super::theme::TEXT
        } else {
            super::theme::MUTED
        },
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            label,
        )
    });
    response.on_hover_text(label)
}
