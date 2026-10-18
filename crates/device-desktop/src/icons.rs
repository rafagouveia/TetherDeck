//! Consistent Lucide-derived application icons and native-size Windows caption glyphs.
//! Lucide geometry is distributed under assets/icons/LICENSE-LUCIDE; stroke is 1.75.
use crate::desktop::theme;
use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, tooltip::Tooltip};
use gpui_kit::*;

pub fn icon(name: IconName) -> Icon {
    let bytes = match name.clone() {
        IconName::Activity => Some(include_bytes!("../assets/icons/activity.svg") as &[u8]),
        IconName::AppWindow => Some(include_bytes!("../assets/icons/app-window.svg") as &[u8]),
        IconName::Blocks => Some(include_bytes!("../assets/icons/blocks.svg") as &[u8]),
        IconName::Bot => Some(include_bytes!("../assets/icons/bot.svg") as &[u8]),
        IconName::Camera => Some(include_bytes!("../assets/icons/camera.svg") as &[u8]),
        IconName::ChartNoAxesCombined => {
            Some(include_bytes!("../assets/icons/chart-no-axes-combined.svg") as &[u8])
        }
        IconName::ChevronDown => Some(include_bytes!("../assets/icons/chevron-down.svg") as &[u8]),
        IconName::Copy => Some(include_bytes!("../assets/icons/copy.svg") as &[u8]),
        IconName::Download => Some(include_bytes!("../assets/icons/download.svg") as &[u8]),
        IconName::ExternalLink => {
            Some(include_bytes!("../assets/icons/external-link.svg") as &[u8])
        }
        IconName::House => Some(include_bytes!("../assets/icons/house.svg") as &[u8]),
        IconName::Layers => Some(include_bytes!("../assets/icons/layers.svg") as &[u8]),
        IconName::Lock => Some(include_bytes!("../assets/icons/lock.svg") as &[u8]),
        IconName::Monitor => Some(include_bytes!("../assets/icons/monitor.svg") as &[u8]),
        IconName::Network => Some(include_bytes!("../assets/icons/network.svg") as &[u8]),
        IconName::Palette => Some(include_bytes!("../assets/icons/palette.svg") as &[u8]),
        IconName::Pause => Some(include_bytes!("../assets/icons/pause.svg") as &[u8]),
        IconName::Play => Some(include_bytes!("../assets/icons/play.svg") as &[u8]),
        IconName::Plug => Some(include_bytes!("../assets/icons/plug.svg") as &[u8]),
        IconName::RefreshCw => Some(include_bytes!("../assets/icons/refresh-cw.svg") as &[u8]),
        IconName::ScanLine => Some(include_bytes!("../assets/icons/scan-line.svg") as &[u8]),
        IconName::Search => Some(include_bytes!("../assets/icons/search.svg") as &[u8]),
        IconName::Server => Some(include_bytes!("../assets/icons/server.svg") as &[u8]),
        IconName::ShieldCheck => Some(include_bytes!("../assets/icons/shield-check.svg") as &[u8]),
        IconName::Smartphone => Some(include_bytes!("../assets/icons/smartphone.svg") as &[u8]),
        IconName::Unplug => Some(include_bytes!("../assets/icons/unplug.svg") as &[u8]),
        IconName::Upload => Some(include_bytes!("../assets/icons/upload.svg") as &[u8]),
        IconName::Usb => Some(include_bytes!("../assets/icons/usb.svg") as &[u8]),
        IconName::Wifi => Some(include_bytes!("../assets/icons/wifi.svg") as &[u8]),
        IconName::Settings2 => Some(include_bytes!("../assets/icons/settings-2.svg") as &[u8]),
        IconName::Volume2 => Some(include_bytes!("../assets/icons/volume-2.svg") as &[u8]),
        IconName::Volume1 => Some(include_bytes!("../assets/icons/volume-1.svg") as &[u8]),
        IconName::MousePointer2 => {
            Some(include_bytes!("../assets/icons/mouse-pointer-2.svg") as &[u8])
        }
        _ => None,
    };
    match bytes {
        Some(bytes) => Icon::default().data(bytes),
        None => Icon::new(name),
    }
}

pub fn panel_left() -> Icon {
    Icon::default()
        .data(include_bytes!("../assets/icons/panel-left.svg"))
        .size(px(18.))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CaptionIcon {
    Minimize,
    Maximize,
    Restore,
    Close,
}

/// Add as a child, not Button::icon: the latter expands it to the control's icon size.
pub fn caption_icon(kind: CaptionIcon) -> Icon {
    let bytes: &[u8] = match kind {
        CaptionIcon::Minimize => include_bytes!("../assets/icons/caption-minimize.svg"),
        CaptionIcon::Maximize => include_bytes!("../assets/icons/caption-maximize.svg"),
        CaptionIcon::Restore => include_bytes!("../assets/icons/caption-restore.svg"),
        CaptionIcon::Close => include_bytes!("../assets/icons/caption-close.svg"),
    };
    Icon::default().data(bytes).size(px(10.))
}

/// GPUI Base owns native button focus, accessibility and one click handler.
/// Drag belongs only to the adjacent caption area; these controls remain client areas.
pub fn caption_button(
    id: impl Into<ElementId>,
    kind: CaptionIcon,
    width: Pixels,
    height: Pixels,
    label: impl Into<SharedString>,
) -> gpui_kit::base::Button {
    let label = label.into();
    let tooltip = label.clone();
    gpui_kit::base::Button::new(id)
        .w(width)
        .min_w(width)
        .h(height)
        .min_h(height)
        .flex_none()
        .tab_stop(false)
        .focusable(false)
        .rounded(px(0.))
        .text_color(theme::text())
        .cursor_default()
        .hover(move |style| {
            if kind == CaptionIcon::Close {
                style.bg(rgb(0xe81123)).text_color(rgb(0xffffff))
            } else {
                style.bg(theme::hover_surface()).text_color(theme::text())
            }
        })
        .active(move |style| {
            if kind == CaptionIcon::Close {
                style.bg(rgb(0xc50f1f)).text_color(rgb(0xffffff))
            } else {
                style.bg(theme::accent().opacity(0.18))
            }
        })
        .accessibility_label(label)
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(caption_icon(kind))
}
