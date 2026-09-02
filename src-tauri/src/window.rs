//! Window position and size persistence.
//!
//! `tauri-plugin-window-state` stores physical pixels. With two displays of
//! different scale factors, the restore is applied while the window still sits
//! on the other display, so the scale factor gets multiplied in every launch
//! and the window keeps growing and drifting. Logical pixels are the same unit
//! across displays on both macOS and Windows, so we store those instead.

use serde::{Deserialize, Serialize};
use std::fs;
use tauri::{LogicalPosition, LogicalSize, Manager, PhysicalPosition, PhysicalSize, Window};

const FILE: &str = "window.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WindowState {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    #[serde(default)]
    maximized: bool,
}

fn path() -> Option<std::path::PathBuf> {
    hitai_core::home_dir().ok().map(|d| d.join(FILE))
}

/// A rectangle in logical pixels.
#[derive(Debug, Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    /// Whether enough of `other` overlaps this rectangle to stay reachable.
    fn covers(&self, other: &Rect) -> bool {
        let overlap_x = (self.x + self.w).min(other.x + other.w) - self.x.max(other.x);
        let overlap_y = (self.y + self.h).min(other.y + other.h) - self.y.max(other.y);
        overlap_x > 80.0 && overlap_y > 40.0
    }
}

fn logical_rect(position: PhysicalPosition<i32>, size: PhysicalSize<u32>, scale: f64) -> Rect {
    let position = position.to_logical::<f64>(scale);
    let size = size.to_logical::<f64>(scale);
    Rect {
        x: position.x,
        y: position.y,
        w: size.width,
        h: size.height,
    }
}

/// Every display, in logical pixels.
fn screens(window: &Window) -> Vec<Rect> {
    window
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| logical_rect(*m.position(), *m.size(), m.scale_factor()))
        .collect()
}

/// Restore the saved geometry. Called once at startup.
pub fn restore(window: &Window) {
    let Some(path) = path() else { return };
    let Ok(raw) = fs::read_to_string(path) else {
        return;
    };
    let Ok(state) = serde_json::from_str::<WindowState>(&raw) else {
        return;
    };

    if state.width > 0.0 && state.height > 0.0 {
        let _ = window.set_size(LogicalSize::new(state.width, state.height));
    }

    // Only move the window somewhere it can actually be seen and grabbed.
    // A display may have been unplugged since the geometry was saved.
    let wanted = Rect {
        x: state.x,
        y: state.y,
        w: state.width,
        h: state.height,
    };
    if screens(window).iter().any(|s| s.covers(&wanted)) {
        let _ = window.set_position(LogicalPosition::new(state.x, state.y));
    }

    if state.maximized {
        let _ = window.maximize();
    }
}

/// Save the current geometry. Called when the window is closing.
pub fn save(window: &Window) {
    let Some(path) = path() else { return };
    let scale = window.scale_factor().unwrap_or(1.0);
    let maximized = window.is_maximized().unwrap_or(false);

    let (Ok(position), Ok(size)) = (window.outer_position(), window.inner_size()) else {
        return;
    };
    let rect = logical_rect(position, size, scale);

    let state = WindowState {
        x: rect.x,
        y: rect.y,
        width: rect.w,
        height: rect.h,
        maximized,
    };
    if let Ok(body) = serde_json::to_string_pretty(&state) {
        let _ = fs::write(path, body);
    }
}

/// Wire persistence into the app.
pub fn attach(app: &tauri::AppHandle) {
    // A webview window is a window; restore works on the window half.
    if let Some(webview) = app.get_webview_window("main") {
        let guard = webview.as_ref().window_ref();
        restore(&guard);
    }
}

/// Handle a window event, saving geometry before the window goes away.
pub fn on_event(window: &Window, event: &tauri::WindowEvent) {
    if matches!(
        event,
        tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
    ) {
        save(window);
    }
}
