//! Windows backend: GetWindowPlacement / SetWindowPlacement.
//!
//! A `WINDOWPLACEMENT` is the shape Win32 itself uses to describe where a
//! window belongs. Its rectangle is the *restored* position even while the
//! window is maximized, and `showCmd` carries the maximized or minimized
//! state, so one struct round-trips everything without extra bookkeeping.
//!
//! The coordinates are workspace relative, which is what makes this better
//! than storing raw screen coordinates: they stay meaningful when the taskbar
//! moves or the resolution changes.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::Window;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowPlacement, SetWindowPlacement, SW_SHOWMINIMIZED, SW_SHOWNORMAL, WINDOWPLACEMENT,
    WINDOWPLACEMENT_FLAGS,
};

const FILE: &str = "window-win32.json";

pub struct Placement;

/// The parts of a WINDOWPLACEMENT worth keeping across runs.
#[derive(Debug, Serialize, Deserialize)]
struct Saved {
    show_cmd: u32,
    flags: u32,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

fn path() -> Result<PathBuf, String> {
    hitai_core::home_dir()
        .map(|d| d.join(FILE))
        .map_err(|e| e.to_string())
}

fn hwnd(window: &Window) -> Result<HWND, String> {
    window
        .hwnd()
        .map_err(|e| format!("창 핸들을 얻지 못했습니다: {e}"))
}

impl super::Memory for Placement {
    fn name(&self) -> &'static str {
        "Win32 창 배치"
    }

    fn usable(&self, window: &Window) -> bool {
        hwnd(window).is_ok()
    }

    fn restore(&self, window: &Window) -> Result<(), String> {
        let path = path()?;
        // Nothing saved yet is the normal first run.
        let Ok(raw) = fs::read_to_string(&path) else {
            return Ok(());
        };
        let saved: Saved =
            serde_json::from_str(&raw).map_err(|e| format!("저장된 배치를 읽지 못했습니다: {e}"))?;

        let hwnd = hwnd(window)?;

        // Start from the window's current placement so every field we do not
        // persist keeps a value Win32 considers valid.
        let mut placement = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        unsafe { GetWindowPlacement(hwnd, &mut placement) }
            .map_err(|e| format!("현재 배치를 읽지 못했습니다: {e}"))?;

        placement.flags = WINDOWPLACEMENT_FLAGS(saved.flags);
        placement.rcNormalPosition = RECT {
            left: saved.left,
            top: saved.top,
            right: saved.right,
            bottom: saved.bottom,
        };

        // Coming back minimized would look like the app failed to start, so a
        // window saved while minimized is restored to its normal shape.
        // `showCmd` is a plain u32 here while the SW_ constants are typed.
        placement.showCmd = if saved.show_cmd == SW_SHOWMINIMIZED.0 as u32 {
            SW_SHOWNORMAL.0 as u32
        } else {
            saved.show_cmd
        };

        unsafe { SetWindowPlacement(hwnd, &placement) }
            .map_err(|e| format!("배치를 적용하지 못했습니다: {e}"))
    }

    fn save(&self, window: &Window) -> Result<(), String> {
        let hwnd = hwnd(window)?;
        let mut placement = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        unsafe { GetWindowPlacement(hwnd, &mut placement) }
            .map_err(|e| format!("배치를 읽지 못했습니다: {e}"))?;

        let saved = Saved {
            show_cmd: placement.showCmd,
            flags: placement.flags.0,
            left: placement.rcNormalPosition.left,
            top: placement.rcNormalPosition.top,
            right: placement.rcNormalPosition.right,
            bottom: placement.rcNormalPosition.bottom,
        };
        let body = serde_json::to_string_pretty(&saved).map_err(|e| e.to_string())?;
        fs::write(path()?, body).map_err(|e| format!("배치를 쓰지 못했습니다: {e}"))
    }
}
