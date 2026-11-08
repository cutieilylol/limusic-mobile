//! Android: the track-change toast is the media notification itself, which the playback service
//! updates on every track (see `media.rs`), so a second notification would only duplicate it.

use tauri::AppHandle;

pub fn track_changed(_app: &AppHandle, _title: &str, _artists: &str, _even_focused: bool) {}
