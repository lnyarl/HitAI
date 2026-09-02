//! macOS backend: NSWindow frame.
//!
//! AppKit's frame autosave (`setFrameAutosaveName:`) was tried first and does
//! not survive this window's lifecycle. Its stored string pairs the frame with
//! the screen frame captured at save time, and reconciling those against the
//! current screens fights Tauri's own placement: a frame saved on the second
//! display came back on the first, and one round trip lost the entry outright.
//!
//! So we read and write `NSWindow.frame` ourselves. An `NSRect` is already in
//! points, a single bottom-left origin coordinate space that spans every
//! display regardless of each display's scale factor. That is what makes this
//! correct across a Retina and a non-Retina screen: there is no scale factor
//! to apply, and no per-screen mapping to get wrong.
//!
//! `constrainFrameRect:toScreen:` with no screen asks AppKit to pull a frame
//! back onto whichever display it best belongs to, which covers a monitor that
//! has been unplugged since the frame was saved.

use objc2::rc::Retained;
use objc2_app_kit::NSWindow;
use objc2_foundation::{NSPoint, NSRect, NSSize};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::Window;

const FILE: &str = "window-macos.json";

#[derive(Default)]
pub struct Frame;

/// An NSWindow frame in points.
#[derive(Debug, Serialize, Deserialize)]
struct Saved {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

fn path() -> Result<PathBuf, String> {
    hitai_core::home_dir()
        .map(|d| d.join(FILE))
        .map_err(|e| e.to_string())
}

/// The NSWindow behind a Tauri window.
fn ns_window(window: &Window) -> Option<Retained<NSWindow>> {
    let ptr = window.ns_window().ok()?;
    if ptr.is_null() {
        return None;
    }
    // Tauri hands out a borrowed pointer; retain it for the call.
    unsafe { Retained::retain(ptr as *mut NSWindow) }
}

impl super::Memory for Frame {
    fn name(&self) -> &'static str {
        "NSWindow 프레임"
    }

    fn usable(&self, window: &Window) -> bool {
        ns_window(window).is_some()
    }

    fn restore(&self, window: &Window) -> Result<(), String> {
        // Nothing saved yet is the normal first run.
        let Ok(raw) = fs::read_to_string(path()?) else {
            return Ok(());
        };
        let saved: Saved =
            serde_json::from_str(&raw).map_err(|e| format!("저장된 프레임을 읽지 못했습니다: {e}"))?;
        if saved.width <= 0.0 || saved.height <= 0.0 {
            return Ok(());
        }

        let ns_window = ns_window(window).ok_or("NSWindow에 접근할 수 없습니다")?;
        let wanted = NSRect::new(
            NSPoint::new(saved.x, saved.y),
            NSSize::new(saved.width, saved.height),
        );

        // Let AppKit decide whether the frame still lands on a display. With
        // no screen argument it picks the best match and pulls the frame back
        // on screen if the display it was saved on is gone.
        let safe = ns_window.constrainFrameRect_toScreen(wanted, None);
        let frame = if safe.size.width > 0.0 && safe.size.height > 0.0 {
            // Keep the saved size; constraining may shrink it to fit a
            // smaller screen, and we would rather keep the shape the user
            // chose than have it creep smaller each launch.
            NSRect::new(safe.origin, wanted.size)
        } else {
            wanted
        };

        ns_window.setFrame_display(frame, true);
        Ok(())
    }

    fn save(&self, window: &Window) -> Result<(), String> {
        let ns_window = ns_window(window).ok_or("NSWindow에 접근할 수 없습니다")?;
        let frame = ns_window.frame();

        let saved = Saved {
            x: frame.origin.x,
            y: frame.origin.y,
            width: frame.size.width,
            height: frame.size.height,
        };
        let body = serde_json::to_string_pretty(&saved).map_err(|e| e.to_string())?;
        fs::write(path()?, body).map_err(|e| format!("프레임을 쓰지 못했습니다: {e}"))
    }
}
