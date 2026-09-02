//! Portable backend: logical pixels in a JSON file.
//!
//! Used where no native backend exists, or when the native window handle
//! cannot be reached. Logical pixels are the unit to store: physical pixels
//! get multiplied by the scale factor again on every restore when two
//! displays have different scale factors, which makes the window grow and
//! drift each launch.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize, Window};

const FILE: &str = "window.json";

pub struct LogicalJson;

#[derive(Debug, Serialize, Deserialize)]
struct Saved {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    #[serde(default)]
    maximized: bool,
}

fn path() -> Result<PathBuf, String> {
    hitai_core::home_dir()
        .map(|d| d.join(FILE))
        .map_err(|e| e.to_string())
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
    /// Whether enough of `other` lands on this rectangle to stay grabbable.
    fn covers(&self, other: &Rect) -> bool {
        let overlap_x = (self.x + self.w).min(other.x + other.w) - self.x.max(other.x);
        let overlap_y = (self.y + self.h).min(other.y + other.h) - self.y.max(other.y);
        overlap_x > 80.0 && overlap_y > 40.0
    }
}

fn to_logical(position: PhysicalPosition<i32>, size: PhysicalSize<u32>, scale: f64) -> Rect {
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
        .map(|m| to_logical(*m.position(), *m.size(), m.scale_factor()))
        .collect()
}

impl super::Memory for LogicalJson {
    fn name(&self) -> &'static str {
        "논리 픽셀 파일"
    }

    fn usable(&self, _window: &Window) -> bool {
        // The last resort, so it always accepts.
        true
    }

    fn restore(&self, window: &Window) -> Result<(), String> {
        // Nothing saved yet is the normal first run.
        let Ok(raw) = fs::read_to_string(path()?) else {
            return Ok(());
        };
        let saved: Saved =
            serde_json::from_str(&raw).map_err(|e| format!("저장된 값을 읽지 못했습니다: {e}"))?;

        if saved.width > 0.0 && saved.height > 0.0 {
            window
                .set_size(LogicalSize::new(saved.width, saved.height))
                .map_err(|e| e.to_string())?;
        }

        // A display may have been unplugged since this was saved. Moving the
        // window off screen would leave it unreachable, so only move it when
        // it still lands somewhere visible.
        let wanted = Rect {
            x: saved.x,
            y: saved.y,
            w: saved.width,
            h: saved.height,
        };
        if screens(window).iter().any(|s| s.covers(&wanted)) {
            window
                .set_position(LogicalPosition::new(saved.x, saved.y))
                .map_err(|e| e.to_string())?;
        }

        if saved.maximized {
            window.maximize().map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn save(&self, window: &Window) -> Result<(), String> {
        let scale = window.scale_factor().map_err(|e| e.to_string())?;
        let position = window.outer_position().map_err(|e| e.to_string())?;
        let size = window.inner_size().map_err(|e| e.to_string())?;
        let rect = to_logical(position, size, scale);

        let saved = Saved {
            x: rect.x,
            y: rect.y,
            width: rect.w,
            height: rect.h,
            maximized: window.is_maximized().unwrap_or(false),
        };
        let body = serde_json::to_string_pretty(&saved).map_err(|e| e.to_string())?;
        fs::write(path()?, body).map_err(|e| format!("저장하지 못했습니다: {e}"))
    }
}
