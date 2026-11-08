//! Android: there is no floating mini window. The phone's own surfaces (the media notification,
//! the lock screen, the in-app mini bar) cover what the desktop widget is for.

use tauri::AppHandle;

pub const LABEL: &str = "mini";

pub fn open(_app: &AppHandle) -> Result<(), String> {
    Err("The mini player isn't available on Android".into())
}

pub fn set_compact(_app: &AppHandle, _compact: bool) -> Result<(), String> {
    Ok(())
}

pub fn save_position(_app: &AppHandle) {}

pub fn close(_app: &AppHandle) {}
