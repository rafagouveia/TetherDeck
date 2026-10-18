//! Native caption actions for our Windows windows and proportional constraints
//! scoped to the remote phone window. This module does not automate UI.
//!
//! GPUI Windows 0.3.9 leaves start_window_move/resize at their no-op defaults.
//! Its DirectComposition renderer already supports premultiplied transparency;
//! do not add WS_EX_LAYERED or replace the renderer to obtain a clear background.
use gpui_kit::{Bounds, Pixels, Point, Window};

/// Actual phone bezel in client coordinates, supplied by GPUI prepaint.
/// The title bar and side controls are deliberately outside the resize target.
pub fn update_resize_bounds(window: &Window, bounds: Bounds<Pixels>) {
    platform::update_resize_bounds(window, bounds);
}

pub fn is_resize_area(window: &Window, position: Point<Pixels>) -> bool {
    platform::is_resize_area(window, position)
}

/// Offsets are total horizontal/vertical chrome inside the CLIENT area, in
/// GPUI logical pixels. Call again when rotation/layout changes the phone ratio.
/// This installs constraints; the view chooses its initial window dimensions.
pub fn configure(
    window: &mut Window,
    phone_aspect: f32,
    horizontal_chrome: f32,
    vertical_chrome: f32,
) -> Result<(), String> {
    platform::configure(window, phone_aspect, horizontal_chrome, vertical_chrome)
}

pub fn minimize(window: &Window) -> Result<(), String> {
    platform::minimize(window)
}
pub fn maximize_restore(window: &Window) -> Result<(), String> {
    platform::maximize_restore(window)
}
pub fn close(window: &Window) -> Result<(), String> {
    platform::close(window)
}

#[cfg(windows)]
mod platform {
    use super::{Bounds, Pixels, Point, Window};
    use gpui_kit::WindowBackgroundAppearance;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::cell::Cell;
    use windows::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::{
            Dwm::{
                DWMNCRP_DISABLED, DWMSBT_NONE, DWMWA_BORDER_COLOR, DWMWA_NCRENDERING_POLICY,
                DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
                DwmSetWindowAttribute,
            },
            Gdi::{
                GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
                ScreenToClient,
            },
        },
        System::{
            LibraryLoader::{GetModuleHandleA, GetProcAddress},
            Threading::GetCurrentThreadId,
        },
        UI::{
            HiDpi::GetDpiForWindow,
            Shell::{DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::*,
        },
    };
    use windows::core::{BOOL, PCSTR};

    const SUBCLASS_ID: usize = 0x5444_4348; // TDCH, scoped to our callback + HWND.

    #[derive(Clone, Copy)]
    struct Geometry {
        aspect: f64,
        horizontal: f64,
        vertical: f64,
    }

    struct ChromeState {
        geometry: Cell<Geometry>,
        native_limits: Cell<MINMAXINFO>,
        resize_bounds: Cell<Option<[f64; 4]>>,
    }

    fn hwnd(window: &Window) -> Result<HWND, String> {
        let handle = HasWindowHandle::window_handle(window)
            .map_err(|_| "A janela não expõe um handle nativo.".to_owned())?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err("A janela remota exige um handle Win32.".into());
        };
        let hwnd = HWND(handle.hwnd.get() as *mut _);
        // Subclass state is owned exclusively by this window's UI thread.
        if unsafe { GetWindowThreadProcessId(hwnd, None) != GetCurrentThreadId() } {
            return Err("O chrome deve ser atualizado na thread da janela.".into());
        }
        Ok(hwnd)
    }

    pub(super) fn configure(
        window: &mut Window,
        aspect: f32,
        horizontal: f32,
        vertical: f32,
    ) -> Result<(), String> {
        if !aspect.is_finite()
            || !(0.1..=10.0).contains(&aspect)
            || !horizontal.is_finite()
            || !(0.0..=4096.0).contains(&horizontal)
            || !vertical.is_finite()
            || !(0.0..=4096.0).contains(&vertical)
        {
            return Err("Proporção ou margens inválidas para a janela remota.".into());
        }
        let hwnd = hwnd(window)?;
        let geometry = Geometry {
            aspect: aspect.into(),
            horizontal: horizontal.into(),
            vertical: vertical.into(),
        };
        // SetWindowSubclass manages its own chain; GPUI's WNDPROC is preserved.
        // https://learn.microsoft.com/windows/win32/controls/subclassing-overview
        let mut reference = 0usize;
        unsafe {
            if GetWindowSubclass(hwnd, Some(chrome_proc), SUBCLASS_ID, Some(&mut reference))
                .as_bool()
            {
                if reference == 0 {
                    return Err("Estado do chrome nativo indisponível.".into());
                }
                (*(reference as *const ChromeState)).geometry.set(geometry);
            } else {
                let reference = Box::into_raw(Box::new(ChromeState {
                    geometry: Cell::new(geometry),
                    native_limits: Cell::new(MINMAXINFO::default()),
                    resize_bounds: Cell::new(None),
                }));
                if !SetWindowSubclass(hwnd, Some(chrome_proc), SUBCLASS_ID, reference as usize)
                    .as_bool()
                {
                    drop(Box::from_raw(reference));
                    return Err("Não foi possível configurar a janela remota.".into());
                }
            }
            // Windows 11: suppress DWM's thin native border. Earlier Windows
            // versions may reject this optional appearance attribute.
            let none = 0xffff_fffeu32;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_BORDER_COLOR,
                &none as *const _ as *const _,
                std::mem::size_of_val(&none) as u32,
            );
        }
        window.set_background_appearance(WindowBackgroundAppearance::Transparent);
        // Transparent in this GPUI backend also enables ACCENT_ENABLE_TRANSPARENTGRADIENT.
        // That system backdrop tints the entire rectangular HWND independently
        // of our alpha-zero DirectComposition surface. Keep the renderer's alpha
        // mode, but remove the native accent, frame, shadow and corner treatment.
        clear_native_decoration(hwnd)?;
        if std::env::var("GPUI_DISABLE_DIRECT_COMPOSITION")
            .is_ok_and(|value| value == "true" || value == "1")
        {
            return Err("A transparência exige DirectComposition; GPUI_DISABLE_DIRECT_COMPOSITION está ativo.".into());
        }
        Ok(())
    }

    fn clear_native_decoration(hwnd: HWND) -> Result<(), String> {
        #[repr(C)]
        struct AccentPolicy {
            state: u32,
            flags: u32,
            color: u32,
            animation: u32,
        }
        #[repr(C)]
        struct CompositionData {
            attribute: u32,
            data: *mut std::ffi::c_void,
            size: usize,
        }
        type SetComposition = unsafe extern "system" fn(HWND, *mut CompositionData) -> BOOL;
        unsafe {
            let user32 = GetModuleHandleA(PCSTR(c"user32.dll".as_ptr().cast()))
                .map_err(|_| "Composição nativa do Windows indisponível.".to_owned())?;
            let function = GetProcAddress(
                user32,
                PCSTR(c"SetWindowCompositionAttribute".as_ptr().cast()),
            )
            .ok_or_else(|| "Composição transparente do Windows indisponível.".to_owned())?;
            let set_composition: SetComposition = std::mem::transmute(function);
            let mut accent = AccentPolicy {
                state: 0,
                flags: 0,
                color: 0,
                animation: 0,
            };
            let mut data = CompositionData {
                attribute: 19,
                data: (&mut accent as *mut AccentPolicy).cast(),
                size: std::mem::size_of_val(&accent),
            };
            if !set_composition(hwnd, &mut data).as_bool() {
                return Err("Não foi possível remover o fundo de composição da janela.".into());
            }
            // Non-client rendering policy is supported since Vista. The other
            // optional attributes may be unavailable on older Windows versions.
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_NCRENDERING_POLICY,
                &DWMNCRP_DISABLED as *const _ as *const std::ffi::c_void,
                std::mem::size_of_val(&DWMNCRP_DISABLED) as u32,
            )
            .map_err(|_| "Não foi possível remover a decoração nativa da janela.".to_owned())?;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                &DWMSBT_NONE as *const _ as *const std::ffi::c_void,
                std::mem::size_of_val(&DWMSBT_NONE) as u32,
            );
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &DWMWCP_DONOTROUND as *const _ as *const std::ffi::c_void,
                std::mem::size_of_val(&DWMWCP_DONOTROUND) as u32,
            );
        }
        Ok(())
    }

    fn with_state<R>(window: &Window, f: impl FnOnce(&ChromeState) -> R) -> Option<R> {
        let hwnd = hwnd(window).ok()?;
        let mut reference = 0;
        unsafe {
            if !GetWindowSubclass(hwnd, Some(chrome_proc), SUBCLASS_ID, Some(&mut reference))
                .as_bool()
                || reference == 0
            {
                return None;
            }
            Some(f(&*(reference as *const ChromeState)))
        }
    }

    pub(super) fn update_resize_bounds(window: &Window, bounds: Bounds<Pixels>) {
        let rect = [
            f64::from(f32::from(bounds.origin.x)),
            f64::from(f32::from(bounds.origin.y)),
            f64::from(f32::from(bounds.origin.x + bounds.size.width)),
            f64::from(f32::from(bounds.origin.y + bounds.size.height)),
        ];
        if rect.iter().all(|n| n.is_finite()) && rect[2] > rect[0] && rect[3] > rect[1] {
            with_state(window, |state| state.resize_bounds.set(Some(rect)));
        }
    }

    fn hit_phone_corner(rect: [f64; 4], x: f64, y: f64) -> Option<u32> {
        // 24px square centered on each bezel corner, including its clear margin.
        const RADIUS: f64 = 12.;
        for (cx, cy, hit) in [
            (rect[0], rect[1], HTTOPLEFT),
            (rect[2], rect[1], HTTOPRIGHT),
            (rect[0], rect[3], HTBOTTOMLEFT),
            (rect[2], rect[3], HTBOTTOMRIGHT),
        ] {
            if (x - cx).abs() <= RADIUS && (y - cy).abs() <= RADIUS {
                return Some(hit);
            }
        }
        None
    }

    pub(super) fn is_resize_area(window: &Window, position: Point<Pixels>) -> bool {
        if hwnd(window).is_ok_and(|hwnd| unsafe { IsZoomed(hwnd).as_bool() }) {
            return false;
        }
        with_state(window, |state| {
            state
                .resize_bounds
                .get()
                .and_then(|rect| {
                    hit_phone_corner(
                        rect,
                        f64::from(f32::from(position.x)),
                        f64::from(f32::from(position.y)),
                    )
                })
                .is_some()
        })
        .unwrap_or(false)
    }

    pub(super) fn minimize(window: &Window) -> Result<(), String> {
        if unsafe { ShowWindowAsync(hwnd(window)?, SW_MINIMIZE).as_bool() } {
            Ok(())
        } else {
            Err("Não foi possível minimizar a janela.".into())
        }
    }

    pub(super) fn maximize_restore(window: &Window) -> Result<(), String> {
        let hwnd = hwnd(window)?;
        let command = if unsafe { IsZoomed(hwnd).as_bool() } {
            SW_RESTORE
        } else {
            SW_MAXIMIZE
        };
        if unsafe { ShowWindowAsync(hwnd, command).as_bool() } {
            Ok(())
        } else {
            Err("Não foi possível alterar o tamanho da janela.".into())
        }
    }

    pub(super) fn close(window: &Window) -> Result<(), String> {
        unsafe { PostMessageW(Some(hwnd(window)?), WM_CLOSE, WPARAM(0), LPARAM(0)) }
            .map_err(|_| "Não foi possível fechar a janela.".into())
    }

    fn physical_extra(hwnd: HWND, geometry: Geometry) -> (f64, f64) {
        let mut outer = RECT::default();
        let mut client = RECT::default();
        let (frame_x, frame_y) = unsafe {
            if GetWindowRect(hwnd, &mut outer).is_ok() && GetClientRect(hwnd, &mut client).is_ok() {
                (
                    (outer.right - outer.left - client.right + client.left).max(0),
                    (outer.bottom - outer.top - client.bottom + client.top).max(0),
                )
            } else {
                (0, 0)
            }
        };
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let scale = f64::from(if dpi == 0 { 96 } else { dpi }) / 96.0;
        (
            geometry.horizontal * scale + f64::from(frame_x),
            geometry.vertical * scale + f64::from(frame_y),
        )
    }

    fn constrain_rect(
        hwnd: HWND,
        geometry: Geometry,
        native_limits: MINMAXINFO,
        corner: u32,
        rect: &mut RECT,
    ) {
        let (extra_x, extra_y) = physical_extra(hwnd, geometry);
        let width = f64::from(rect.right - rect.left).max(extra_x + 1.0) - extra_x;
        let height = f64::from(rect.bottom - rect.top).max(extra_y + 1.0) - extra_y;
        // Project the proposed phone rectangle onto w=aspect*h, preserving the
        // opposite corner. This supports horizontal and vertical pointer motion.
        let mut limits = native_limits;
        constrain_tracking_limits(hwnd, geometry, &mut limits);
        let minimum = (f64::from(limits.ptMinTrackSize.y) - extra_y)
            .max((f64::from(limits.ptMinTrackSize.x) - extra_x) / geometry.aspect)
            .max(1.0);
        let maximum = (f64::from(limits.ptMaxTrackSize.y) - extra_y)
            .min((f64::from(limits.ptMaxTrackSize.x) - extra_x) / geometry.aspect)
            .max(minimum);
        let phone_height = ((geometry.aspect * width + height)
            / (geometry.aspect * geometry.aspect + 1.0))
            .clamp(minimum, maximum);
        let new_width = (phone_height * geometry.aspect + extra_x)
            .round()
            .clamp(1.0, i32::MAX as f64) as i32;
        let new_height = (phone_height + extra_y).round().clamp(1.0, i32::MAX as f64) as i32;
        if matches!(corner, WMSZ_TOPLEFT | WMSZ_BOTTOMLEFT) {
            rect.left = rect.right.saturating_sub(new_width);
        } else {
            rect.right = rect.left.saturating_add(new_width);
        }
        if matches!(corner, WMSZ_TOPLEFT | WMSZ_TOPRIGHT) {
            rect.top = rect.bottom.saturating_sub(new_height);
        } else {
            rect.bottom = rect.top.saturating_add(new_height);
        }
    }

    fn constrain_tracking_limits(hwnd: HWND, geometry: Geometry, limits: &mut MINMAXINFO) {
        let (extra_x, extra_y) = physical_extra(hwnd, geometry);
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let found = unsafe {
            GetMonitorInfoW(
                MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            )
            .as_bool()
        };
        let max_width = if found {
            f64::from(monitor.rcWork.right - monitor.rcWork.left)
        } else {
            f64::from(limits.ptMaxTrackSize.x.max(1))
        };
        let max_height = if found {
            f64::from(monitor.rcWork.bottom - monitor.rcWork.top)
        } else {
            f64::from(limits.ptMaxTrackSize.y.max(1))
        };
        let max_phone_height = (max_height - extra_y)
            .min((max_width - extra_x) / geometry.aspect)
            .max(1.0);
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
        // Reserve the view's vertical controls in landscape, while allowing a
        // smaller floor when the current monitor cannot accommodate that size.
        let client_floor = 496.0 * f64::from(dpi) / 96.0;
        let min_phone_height = (f64::from(limits.ptMinTrackSize.y) - extra_y)
            .max((f64::from(limits.ptMinTrackSize.x) - extra_x) / geometry.aspect)
            .max(client_floor - geometry.vertical * f64::from(dpi) / 96.0)
            .max(1.0)
            .min(max_phone_height);
        limits.ptMinTrackSize.x = (min_phone_height * geometry.aspect + extra_x).ceil() as i32;
        limits.ptMinTrackSize.y = (min_phone_height + extra_y).ceil() as i32;
        limits.ptMaxTrackSize.x = (max_phone_height * geometry.aspect + extra_x).floor() as i32;
        limits.ptMaxTrackSize.y = (max_phone_height + extra_y).floor() as i32;
        if found {
            // Track limits alone constrain the maximized size but leave Windows'
            // default top-left position, making a narrow phone hug the monitor's
            // edge. Set both dimensions and position for the aspect-fitted size.
            // Positions here are relative to this monitor, including work-area
            // insets from taskbars on any side, not virtual-desktop coordinates.
            limits.ptMaxSize = limits.ptMaxTrackSize;
            limits.ptMaxPosition = POINT {
                x: monitor.rcWork.left - monitor.rcMonitor.left
                    + ((max_width as i32 - limits.ptMaxSize.x).max(0) / 2),
                y: monitor.rcWork.top - monitor.rcMonitor.top
                    + ((max_height as i32 - limits.ptMaxSize.y).max(0) / 2),
            };
        }
    }

    fn corner_hit(hwnd: HWND, lparam: LPARAM, reference: usize) -> Option<u32> {
        if unsafe { IsZoomed(hwnd).as_bool() } {
            return None;
        }
        if reference == 0 {
            return None;
        }
        let rect = unsafe { (*(reference as *const ChromeState)).resize_bounds.get() }?;
        let mut cursor = POINT {
            x: (lparam.0 as u16 as i16) as i32,
            y: ((lparam.0 >> 16) as u16 as i16) as i32,
        };
        if !unsafe { ScreenToClient(hwnd, &mut cursor) }.as_bool() {
            return None;
        }
        let scale = f64::from(unsafe { GetDpiForWindow(hwnd) }.max(96)) / 96.;
        hit_phone_corner(
            rect,
            f64::from(cursor.x) / scale,
            f64::from(cursor.y) / scale,
        )
    }

    unsafe extern "system" fn chrome_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        subclass: usize,
        reference: usize,
    ) -> LRESULT {
        // The allocation belongs to this HWND and is freed exactly once, after
        // removing our callback. Never intercept or replace GPUI's lifetime data.
        if message == WM_NCDESTROY {
            unsafe {
                let _ = RemoveWindowSubclass(hwnd, Some(chrome_proc), subclass);
                if reference != 0 {
                    drop(Box::from_raw(reference as *mut ChromeState));
                }
                return DefSubclassProc(hwnd, message, wparam, lparam);
            }
        }
        let geometry = if reference == 0 {
            None
        } else {
            Some(unsafe { (*(reference as *const ChromeState)).geometry.get() })
        };
        if message == WM_SYSCOMMAND
            && wparam.0 as u32 & 0xfff0 == SC_SIZE
            && !matches!(
                wparam.0 as u32 & 0xf,
                WMSZ_TOPLEFT | WMSZ_TOPRIGHT | WMSZ_BOTTOMLEFT | WMSZ_BOTTOMRIGHT
            )
        {
            return LRESULT(0);
        }
        if message == WM_SIZING && lparam.0 != 0 {
            if let Some(geometry) = geometry {
                let rect = unsafe { &mut *(lparam.0 as *mut RECT) };
                if matches!(
                    wparam.0 as u32,
                    WMSZ_TOPLEFT | WMSZ_TOPRIGHT | WMSZ_BOTTOMLEFT | WMSZ_BOTTOMRIGHT
                ) {
                    let limits =
                        unsafe { (*(reference as *const ChromeState)).native_limits.get() };
                    constrain_rect(hwnd, geometry, limits, wparam.0 as u32, rect);
                } else {
                    let _ = unsafe { GetWindowRect(hwnd, rect) };
                }
                return LRESULT(1);
            }
        }
        if message == WM_NCHITTEST {
            if let Some(corner) = corner_hit(hwnd, lparam, reference) {
                return LRESULT(corner as isize);
            }
            let hit = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
            if matches!(
                hit.0 as u32,
                HTLEFT
                    | HTRIGHT
                    | HTTOP
                    | HTBOTTOM
                    | HTBORDER
                    | HTTOPLEFT
                    | HTTOPRIGHT
                    | HTBOTTOMLEFT
                    | HTBOTTOMRIGHT
            ) {
                return LRESULT(HTCLIENT as isize);
            }
            return hit;
        }
        if message == WM_GETMINMAXINFO && lparam.0 != 0 {
            // First preserve GPUI's configured minimum, then project its
            // tracking range onto the same phone aspect as WM_SIZING.
            let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
            if let Some(geometry) = geometry {
                let native_limits = unsafe { *(lparam.0 as *const MINMAXINFO) };
                unsafe {
                    (*(reference as *const ChromeState))
                        .native_limits
                        .set(native_limits);
                }
                constrain_tracking_limits(hwnd, geometry, unsafe {
                    &mut *(lparam.0 as *mut MINMAXINFO)
                });
            }
            return result;
        }
        // GPUI owns DPI, native min/max bounds, fullscreen, and close callbacks.
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{Bounds, Pixels, Point, Window};
    pub(super) fn update_resize_bounds(_: &Window, _: Bounds<Pixels>) {}
    pub(super) fn is_resize_area(_: &Window, _: Point<Pixels>) -> bool {
        false
    }
    fn unsupported() -> Result<(), String> {
        Err("Este chrome nativo está disponível no Windows.".into())
    }
    pub(super) fn configure(_: &mut Window, _: f32, _: f32, _: f32) -> Result<(), String> {
        unsupported()
    }
    pub(super) fn minimize(_: &Window) -> Result<(), String> {
        unsupported()
    }
    pub(super) fn maximize_restore(_: &Window) -> Result<(), String> {
        unsupported()
    }
    pub(super) fn close(_: &Window) -> Result<(), String> {
        unsupported()
    }
}
