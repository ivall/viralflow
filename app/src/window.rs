//! The frameless window: dragging, resizing and the caption buttons.

use slint::winit_030::winit::window::ResizeDirection;
use slint::winit_030::WinitWindowAccessor;
use slint::ComponentHandle;

use crate::{AppState, AppWindow};

pub fn install(window: &AppWindow) {
    let state = window.global::<AppState>();

    let weak = window.as_weak();
    state.on_drag_window(move || {
        if let Some(window) = weak.upgrade() {
            window.window().with_winit_window(|winit| {
                let _ = winit.drag_window();
            });
        }
    });

    let weak = window.as_weak();
    state.on_minimize(move || {
        if let Some(window) = weak.upgrade() {
            window.window().set_minimized(true);
        }
    });

    let weak = window.as_weak();
    state.on_toggle_maximize(move || {
        if let Some(window) = weak.upgrade() {
            let maximized = !window.window().is_maximized();
            window.window().set_maximized(maximized);
            window.global::<AppState>().set_maximized(maximized);
        }
    });

    let weak = window.as_weak();
    state.on_close(move || {
        if let Some(window) = weak.upgrade() {
            let _ = window.window().dispatch_event_with_result(slint::platform::WindowEvent::CloseRequested);
        }
    });

    let weak = window.as_weak();
    state.on_resize(move |direction| {
        let direction = match direction {
            0 => ResizeDirection::North,
            1 => ResizeDirection::NorthEast,
            2 => ResizeDirection::East,
            3 => ResizeDirection::SouthEast,
            4 => ResizeDirection::South,
            5 => ResizeDirection::SouthWest,
            6 => ResizeDirection::West,
            _ => ResizeDirection::NorthWest,
        };
        if let Some(window) = weak.upgrade() {
            window.window().with_winit_window(|winit| {
                let _ = winit.drag_resize_window(direction);
            });
        }
    });

    // Keep the caption button in sync when the system maximizes (snap, double-click).
    let weak = window.as_weak();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(250), move || {
        if let Some(window) = weak.upgrade() {
            let maximized = window.window().is_maximized();
            if window.global::<AppState>().get_maximized() != maximized {
                window.global::<AppState>().set_maximized(maximized);
            }
        }
    });
    std::mem::forget(timer);
}

/// Development aid: with `VF_SNAPSHOT=<file.png>` the window saves a picture of
/// itself after `VF_SNAPSHOT_DELAY` ms (default 1500) and the app quits.
pub fn snapshot_if_requested(window: &AppWindow) {
    let Ok(path) = std::env::var("VF_SNAPSHOT") else { return };
    let delay = std::env::var("VF_SNAPSHOT_DELAY").ok().and_then(|v| v.parse().ok()).unwrap_or(1500);
    let weak = window.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(delay), move || {
        let Some(window) = weak.upgrade() else { return };
        match window.window().take_snapshot() {
            Ok(buffer) => {
                if let Err(error) = save_png(&path, buffer.width(), buffer.height(), buffer.as_bytes()) {
                    log::error!("snapshot failed: {error}");
                }
            }
            Err(error) => log::error!("snapshot failed: {error}"),
        }
        let _ = slint::quit_event_loop();
    });
}

fn save_png(path: &str, width: u32, height: u32, rgba: &[u8]) -> std::io::Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(std::io::Error::other)?;
    writer.write_image_data(rgba).map_err(std::io::Error::other)?;
    Ok(())
}
