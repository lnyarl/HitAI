//! Window geometry memory.
//!
//! Remembering where a window was is a job the operating system already does
//! better than portable code can. AppKit persists a frame and reconciles it
//! against the current displays; Win32 stores a placement in workspace
//! coordinates that survives resolution changes and carries the
//! maximized/minimized state. Both beat storing raw coordinates ourselves.
//!
//! Each platform lives in its own file behind the [`Memory`] trait, so the
//! backends never see each other. A portable backend stands in when no native
//! one is available, or when the native handle cannot be reached.
//!
//! ```text
//! window/
//!   mod.rs        this file: the trait, backend selection, the public API
//!   appkit.rs     macOS   - NSWindow frame autosave
//!   win32.rs      Windows - GetWindowPlacement / SetWindowPlacement
//!   portable.rs   anything else - logical pixels in a JSON file
//! ```

mod portable;

#[cfg(target_os = "macos")]
mod appkit;

#[cfg(target_os = "windows")]
mod win32;

use std::sync::OnceLock;
use tauri::Window;

/// One way of remembering a window's geometry.
///
/// Implementations own their storage. Nothing outside a backend may read or
/// write what that backend persists.
pub trait Memory: Send + Sync {
    /// Short name for logs.
    fn name(&self) -> &'static str;

    /// Whether this backend can work with this window on this machine.
    /// Checked once, before the backend is chosen.
    fn usable(&self, window: &Window) -> bool;

    /// Put the window back where it was. A window with nothing saved yet is
    /// left alone, which is success, not failure.
    fn restore(&self, window: &Window) -> Result<(), String>;

    /// Remember where the window is now.
    fn save(&self, window: &Window) -> Result<(), String>;
}

/// Backends in order of preference. The first usable one wins.
fn candidates() -> Vec<Box<dyn Memory>> {
    let mut out: Vec<Box<dyn Memory>> = Vec::new();

    #[cfg(target_os = "macos")]
    out.push(Box::new(appkit::Frame::default()));

    #[cfg(target_os = "windows")]
    out.push(Box::new(win32::Placement));

    out.push(Box::new(portable::LogicalJson));
    out
}

/// The chosen backend. Decided once, so save and restore never disagree.
fn active(window: &Window) -> &'static dyn Memory {
    static ACTIVE: OnceLock<Box<dyn Memory>> = OnceLock::new();
    ACTIVE
        .get_or_init(|| {
            let mut candidates = candidates();
            let index = candidates.iter().position(|c| c.usable(window));
            match index {
                Some(index) => candidates.swap_remove(index),
                // The portable backend is always last and always usable, so
                // this only happens if the list is somehow empty.
                None => Box::new(portable::LogicalJson),
            }
        })
        .as_ref()
}

/// Restore the window's geometry, once per run.
///
/// Runs on the window's first focus rather than at startup. A backend that
/// asks the platform to place the window needs the window to already belong
/// to a screen, otherwise the platform maps the saved frame against the
/// wrong one and the window lands somewhere else.
fn restore_once(window: &Window) {
    static DONE: OnceLock<()> = OnceLock::new();
    if DONE.set(()).is_err() {
        return;
    }
    let memory = active(window);
    if let Err(e) = memory.restore(window) {
        eprintln!("[HitAI] 창 위치를 복원하지 못했습니다 ({}): {e}", memory.name());
    }
}

/// React to window events.
///
/// Saving happens on `CloseRequested`, while the window still exists. By
/// `Destroyed` the platform handle is already gone and there is nothing left
/// to read a geometry from.
pub fn on_event(window: &Window, event: &tauri::WindowEvent) {
    match event {
        // The window belongs to a screen by the time it is focused.
        tauri::WindowEvent::Focused(true) => restore_once(window),
        tauri::WindowEvent::CloseRequested { .. } => {
            let memory = active(window);
            if let Err(e) = memory.save(window) {
                eprintln!("[HitAI] 창 위치를 저장하지 못했습니다 ({}): {e}", memory.name());
            }
        }
        _ => {}
    }
}
