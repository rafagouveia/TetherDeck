//! Non-secret window placement, independent from connection and UI preferences.
use gpui_kit::*;
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::Duration;

fn path() -> Option<std::path::PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|root| std::path::PathBuf::from(root).join("TetherDeck/window-state.json"))
}
fn state() -> &'static Mutex<serde_json::Value> {
    static STATE: OnceLock<Mutex<serde_json::Value>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(
            path()
                .and_then(|p| std::fs::read(p).ok())
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .filter(|v: &serde_json::Value| v.is_object())
                .unwrap_or_else(|| serde_json::json!({})),
        )
    })
}

pub fn restore(key: &str, default: Size<Pixels>, minimum: Size<Pixels>, cx: &App) -> WindowBounds {
    let fallback = || WindowBounds::Windowed(Bounds::centered(None, default, cx));
    let Ok(saved) = state().lock() else {
        return fallback();
    };
    let value = &saved[key];
    let Some(values) = value["rect"].as_array().filter(|v| v.len() == 4) else {
        return fallback();
    };
    let Some(rect) = values
        .iter()
        .map(|v| v.as_f64().map(|n| n as f32))
        .collect::<Option<Vec<_>>>()
    else {
        return fallback();
    };
    if rect.iter().any(|n| !n.is_finite())
        || rect[2] < f32::from(minimum.width)
        || rect[3] < f32::from(minimum.height)
    {
        return fallback();
    }
    let mut bounds = Bounds::new(
        point(px(rect[0]), px(rect[1])),
        size(px(rect[2]), px(rect[3])),
    );
    // A removed monitor or changed display scale must not strand the title bar.
    let displays = cx.displays();
    let display = displays
        .iter()
        .find(|d| d.bounds().contains(&bounds.origin))
        .or_else(|| displays.first());
    if let Some(display) = display {
        let screen = display.bounds();
        bounds.size.width = bounds.size.width.min(screen.size.width);
        bounds.size.height = bounds.size.height.min(screen.size.height - px(48.));
        bounds.origin.x = bounds
            .origin
            .x
            .max(screen.origin.x)
            .min(screen.origin.x + screen.size.width - bounds.size.width);
        bounds.origin.y = bounds
            .origin
            .y
            .max(screen.origin.y)
            .min(screen.origin.y + screen.size.height - bounds.size.height - px(32.));
    }
    if value["maximized"].as_bool() == Some(true) {
        WindowBounds::Maximized(bounds)
    } else {
        WindowBounds::Windowed(bounds)
    }
}

fn remember(key: &'static str, window: &Window) {
    let placement = window.window_bounds();
    let b = placement.get_bounds();
    let rect = [
        f32::from(b.origin.x),
        f32::from(b.origin.y),
        f32::from(b.size.width),
        f32::from(b.size.height),
    ];
    if rect.iter().any(|n| !n.is_finite()) || rect[2] < 200. || rect[3] < 200. {
        return;
    }
    if let Ok(mut value) = state().lock() {
        value[key] = serde_json::json!({"rect":rect,"maximized":matches!(placement, WindowBounds::Maximized(_))});
    }
    static WRITER: OnceLock<Option<mpsc::SyncSender<()>>> = OnceLock::new();
    let writer = WRITER.get_or_init(|| {
        let (send, receive) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("tetherdeck-window-state".into())
            .spawn(move || {
                while receive.recv().is_ok() {
                    // One bounded wakeup and one latest state; no I/O on the UI thread.
                    while receive.recv_timeout(Duration::from_millis(250)).is_ok() {}
                    save_latest();
                }
            })
            .ok()
            .map(|_| send)
    });
    if let Some(writer) = writer {
        let _ = writer.try_send(());
    }
}

fn save_latest() {
    static WRITE_LOCK: Mutex<()> = Mutex::new(());
    let Ok(_guard) = WRITE_LOCK.lock() else {
        return;
    };
    let Some(path) = path() else { return };
    let bytes = state()
        .lock()
        .ok()
        .and_then(|value| serde_json::to_vec_pretty(&*value).ok());
    if let Some(bytes) = bytes {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let pending = path.with_extension("pending");
        if std::fs::write(&pending, bytes).is_ok() {
            if let Err(error) = crate::settings::replace_file(&pending, &path) {
                eprintln!("TetherDeck: window placement: {error}");
            }
        }
    }
}

pub fn flush(cx: &App) -> Task<()> {
    cx.background_executor().spawn(async {
        save_latest();
    })
}

pub fn observe<T: 'static>(
    key: &'static str,
    window: &mut Window,
    cx: &Context<T>,
) -> Subscription {
    cx.observe_window_bounds(window, move |_, window, _| remember(key, window))
}
