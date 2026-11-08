//! Android media controls: the same [`MediaHandle`] the desktop build feeds MPRIS/SMTC with, here
//! feeding the Media3 session in `PlaybackService.kt`. That session is what draws the media
//! notification and the lock-screen player, and what headset, Bluetooth, Android Auto and Wear OS
//! buttons talk to.
//!
//! Metadata goes out over the JNI bridge; button presses come back as `remote` events and take the
//! same [`AppState`] paths the desktop media keys do, so the queue stays the one source of truth
//! (a "next" from the notification is the app's next, with shuffle, radio and autoplay intact).

use std::sync::{Arc, OnceLock};

use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::state::AppState;
use player::bridge;

pub struct MediaHandle;

impl MediaHandle {
    pub fn set_metadata(
        &self,
        title: &str,
        artist: &str,
        album: Option<&str>,
        cover: Option<&str>,
    ) {
        bridge::send(
            "media.metadata",
            json!({ "title": title, "artist": artist, "album": album, "cover": cover }),
        );
    }

    pub fn set_album(&self, album: &str) {
        bridge::send("media.album", json!({ "album": album }));
    }

    /// The session takes the duration from ExoPlayer itself.
    pub fn set_duration(&self, _secs: f64) {}

    /// Likewise playing state and position: the session reads them off the player it wraps.
    pub fn set_playback(&self, _playing: bool, _pos: f64) {}

    pub fn set_volume(&self, _volume: i64) {}
}

pub fn spawn(app: AppHandle) -> Option<MediaHandle> {
    static APP: OnceLock<AppHandle> = OnceLock::new();
    let _ = APP.set(app);
    bridge::on("remote", |ev| {
        if let Some(app) = APP.get() {
            handle_remote(app, ev.clone());
        }
    });
    Some(MediaHandle)
}

/// A button press from the media session: notification, lock screen, headset, Bluetooth.
fn handle_remote(app: &AppHandle, ev: Value) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(state) = app.try_state::<Arc<AppState>>() else { return };
        let state = state.inner().clone();
        match ev.get("action").and_then(Value::as_str).unwrap_or_default() {
            "play" | "toggle" => state.resume_or_toggle().await,
            "pause" | "stop" => {
                let _ = state.player.pause();
            }
            "next" => state.next_in_queue().await,
            "prev" => state.prev_in_queue().await,
            "seek" => {
                if let Some(pos) = ev.get("pos").and_then(Value::as_f64) {
                    let _ = state.user_seek(pos.max(0.0)).await;
                }
            }
            _ => {}
        }
    });
}

/// Same artwork sizing as the desktop build (the session's bitmap loader fetches this URL).
pub(crate) fn cover_url(url: &str) -> String {
    static WH: OnceLock<regex::Regex> = OnceLock::new();
    static S: OnceLock<regex::Regex> = OnceLock::new();
    let wh = WH.get_or_init(|| regex::Regex::new(r"=w\d+-h\d+").expect("static regex"));
    let s = S.get_or_init(|| regex::Regex::new(r"=s\d+").expect("static regex"));
    if wh.is_match(url) {
        wh.replace(url, "=w544-h544").into_owned()
    } else if s.is_match(url) {
        s.replace(url, "=s544").into_owned()
    } else {
        url.to_owned()
    }
}
