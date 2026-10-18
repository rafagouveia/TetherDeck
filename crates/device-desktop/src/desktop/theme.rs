use crate::settings::ThemePreference;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;
use std::cell::Cell;
thread_local! { static DARK: Cell<bool> = const { Cell::new(true) }; }
pub fn is_dark() -> bool {
    DARK.with(Cell::get)
}
fn palette(dark: u32, light: u32) -> Hsla {
    color(if is_dark() { dark } else { light })
}

// Roles adapted from Zed's upstream One Dark theme, rather than its branding:
// https://github.com/zed-industries/zed/blob/main/assets/themes/one/one.json

pub fn color(hex: u32) -> Hsla {
    rgb(hex).into()
}
pub fn background() -> Hsla {
    palette(0x282c33, 0xeaedf2)
}
pub fn surface() -> Hsla {
    palette(0x2f343e, 0xf2f4f7)
}
pub fn sidebar() -> Hsla {
    palette(0x2f343e, 0xf2f4f7)
}
pub fn chrome() -> Hsla {
    palette(0x3b414d, 0xd9dee6)
}
pub fn button_surface() -> Hsla {
    palette(0x424b5b, 0xffffff)
}
pub fn hover_surface() -> Hsla {
    palette(0x4c5666, 0xe4eaf2)
}
pub fn border() -> Hsla {
    palette(0x363c46, 0xc4cbd5)
}
pub fn muted() -> Hsla {
    palette(0xa9afbc, 0x5f6978)
}
pub fn accent() -> Hsla {
    palette(0x74ade8, 0x2f6faf)
}
pub fn text() -> Hsla {
    palette(0xdce0e5, 0x242a33)
}
pub fn success() -> Hsla {
    palette(0xa1c181, 0x3c7132)
}
pub fn warning() -> Hsla {
    palette(0xdec184, 0x92600f)
}
pub fn error() -> Hsla {
    palette(0xd07277, 0xb83845)
}

pub fn apply(preference: ThemePreference, appearance: WindowAppearance, cx: &mut App) {
    let dark = match preference {
        ThemePreference::Dark => true,
        ThemePreference::Light => false,
        ThemePreference::System => matches!(
            appearance,
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        ),
    };
    DARK.with(|value| value.set(dark));
    Theme::change(
        if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        None,
        cx,
    );
    Theme::update(cx, |theme| {
        theme.font_family = "Segoe UI".into();
        // The kit's rem scale assumes 16px; body content explicitly uses 14px.
        // This keeps native medium button text at 14px and icons at 16px.
        theme.font_size = px(16.);
        theme.radius = px(6.);
        theme.radius_lg = px(6.);
        theme.shadow = false;
        theme.colors.background = background();
        theme.colors.foreground = text();
        theme.colors.border = border();
        theme.colors.muted = surface();
        theme.colors.muted_foreground = muted();
        theme.colors.accent = palette(0x454a56, 0xdbe1e9);
        theme.colors.accent_foreground = text();
        theme.colors.input = palette(0x363c46, 0xc4cbd5);
        theme.colors.group_box = surface();
        theme.colors.group_box_foreground = text();
        theme.colors.primary = palette(0x3a4d66, 0xdbe9f9);
        theme.colors.primary_hover = palette(0x435b78, 0xcbdff5);
        theme.colors.primary_active = palette(0x34465f, 0xbcd3ee);
        theme.colors.primary_foreground = palette(0xc2dcf8, 0x1c4d7c);
        theme.colors.button_primary = theme.colors.primary;
        theme.colors.button_primary_hover = theme.colors.primary_hover;
        theme.colors.button_primary_active = theme.colors.primary_active;
        theme.colors.button_primary_foreground = theme.colors.primary_foreground;
        theme.colors.button = button_surface();
        theme.colors.button_foreground = text();
        theme.colors.button_hover = hover_surface();
        theme.colors.button_active = palette(0x454a56, 0xdbe1e9);
        theme.colors.secondary = surface();
        theme.colors.secondary_foreground = text();
        theme.colors.sidebar = sidebar();
        theme.colors.sidebar_foreground = muted();
        theme.colors.sidebar_accent = palette(0x454a56, 0xdbe1e9);
        theme.colors.sidebar_accent_foreground = text();
        theme.colors.sidebar_border = border();
        theme.colors.sidebar_primary = accent();
        theme.colors.sidebar_primary_foreground = text();
        theme.colors.ring = accent();
        theme.colors.success = success();
        theme.colors.warning = warning();
        theme.colors.danger = error();
    });
}
