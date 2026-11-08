//! The phone player: [`Player`] with the libmpv build's API, driving the decks on the native side
//! (`PlayerEngine.kt`, Media3 ExoPlayer; `PlayerEngine.swift`, AVFoundation).
//!
//! Each engine mirrors `mpv.rs` behaviour for behaviour, so `state.rs` and the orchestrator run
//! unchanged on top of it:
//! - the lookahead goes into the deck's own playlist for a gapless transition, or onto the second
//!   deck when crossfading (equal-power ramp, `TrackEnded` at the *start* of the overlap);
//! - the pause flag survives a load, exactly like mpv's `pause` property, so `load` + `play` is
//!   the same two-step it is on desktop;
//! - repeat-one restarts the item without a transition event.
//!
//! Every query here (`is_idle`, `volume`, `current_path`) answers from a mirror kept current by
//! the engine's events, never by asking the native side: a round trip to the main thread from the
//! async event pump would stall it for the same reason mpv's synchronous `get_property` does.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

#[path = "bridge.rs"]
pub mod bridge;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("player: {0}")]
    Bridge(String),
    /// iOS only for now; ExoPlayer always has a pitch shifter.
    #[error("Pitch shifting isn't available in this build")]
    NoPitchFilter,
}

/// Events pumped from the engine. Same meaning as the desktop ones, see `mpv.rs`.
#[derive(Debug, Clone)]
pub enum PlayerEvent {
    Position(f64),
    Duration(f64),
    Playing(bool),
    Paused(bool),
    TrackEnded,
    TrackFailed(String),
    AudioDeviceLost,
    LookaheadFailed(String),
    VideoFailed(String),
    Error(String),
}

/// No hardware decoding hand-off on Android; the type exists so the shared signatures compile.
pub enum GlDisplay {
    X11(*const c_void),
    Wayland(*const c_void),
}

/// The music video is drawn by the webview's own `<video>` on Android, so there is never a
/// picture to sample here. Kept for the shared `ambient_frame` signature.
pub struct Thumbnail {
    pub pts: f64,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

struct Mirror {
    tx: UnboundedSender<PlayerEvent>,
    idle: AtomicBool,
    volume: AtomicI64,
    path: Mutex<Option<String>>,
}

pub struct Player {
    mirror: Arc<Mirror>,
    events: Option<UnboundedReceiver<PlayerEvent>>,
}

fn send(cmd: &str, args: Value) {
    bridge::send(cmd, args);
}

impl Player {
    /// `cache_dir` is handed to the engine for ExoPlayer's on-disk cache of audio bytes.
    pub fn new(cache_dir: &str) -> Result<Self, Error> {
        let (tx, rx) = unbounded_channel();
        let mirror = Arc::new(Mirror {
            tx,
            idle: AtomicBool::new(true),
            volume: AtomicI64::new(100),
            path: Mutex::new(None),
        });
        let m = mirror.clone();
        bridge::on("player", move |ev| on_event(&m, ev));
        send("player.init", json!({ "cacheDir": cache_dir }));
        Ok(Player { mirror, events: Some(rx) })
    }

    pub fn take_events(&mut self) -> Option<UnboundedReceiver<PlayerEvent>> {
        self.events.take()
    }

    /// Load and play a fresh URL, replacing the playlist. The pause state is left as it is: the
    /// caller's `play()` is what starts it, as with mpv.
    pub fn load(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        gain_db: Option<f64>,
        start: Option<f64>,
    ) -> Result<(), Error> {
        *self.mirror.path.lock().unwrap() = Some(url.to_owned());
        self.mirror.idle.store(false, Ordering::SeqCst);
        let start = start.filter(|p| p.is_finite() && *p > 0.0);
        send(
            "player.load",
            json!({ "url": url, "headers": headers, "gain": gain_db, "start": start }),
        );
        Ok(())
    }

    /// Append the next track: the gapless lookahead, or the crossfade preload.
    pub fn enqueue(&self, url: &str) -> Result<(), Error> {
        send("player.enqueue", json!({ "url": url }));
        Ok(())
    }

    pub fn set_crossfade(&self, secs: Option<f64>) {
        let secs = secs.filter(|s| s.is_finite() && *s >= 1.0).map(|s| s.min(10.0));
        send("player.crossfade", json!({ "secs": secs.unwrap_or(0.0) }));
    }

    /// Drop everything queued after the current item (mpv's `playlist-clear` keeps the current).
    pub fn clear_playlist(&self) -> Result<(), Error> {
        send("player.clearPlaylist", Value::Null);
        Ok(())
    }

    /// Stop outright: nothing loaded, idle until the next `load`.
    pub fn stop(&self) -> Result<(), Error> {
        self.mirror.idle.store(true, Ordering::SeqCst);
        *self.mirror.path.lock().unwrap() = None;
        send("player.stop", Value::Null);
        Ok(())
    }

    pub fn is_idle(&self) -> bool {
        self.mirror.idle.load(Ordering::SeqCst)
    }

    pub fn play(&self) -> Result<(), Error> {
        send("player.play", Value::Null);
        Ok(())
    }

    pub fn pause(&self) -> Result<(), Error> {
        send("player.pause", Value::Null);
        Ok(())
    }

    pub fn toggle(&self) -> Result<(), Error> {
        send("player.toggle", Value::Null);
        Ok(())
    }

    pub fn set_loop_file(&self, on: bool) -> Result<(), Error> {
        send("player.loop", json!({ "on": on }));
        Ok(())
    }

    pub fn seek(&self, position_secs: f64) -> Result<(), Error> {
        send("player.seek", json!({ "pos": position_secs.max(0.0) }));
        Ok(())
    }

    /// Slider percent 0-100, on the same perceptual curve as the desktop build (see
    /// [`perceptual_gain`]).
    pub fn set_volume(&self, volume: i64) -> Result<(), Error> {
        let volume = volume.clamp(0, 100);
        self.mirror.volume.store(volume, Ordering::Relaxed);
        send("player.volume", json!({ "gain": perceptual_gain(volume) }));
        Ok(())
    }

    pub fn volume(&self) -> i64 {
        self.mirror.volume.load(Ordering::Relaxed)
    }

    /// The audio bytes reach ExoPlayer through the loopback audio proxy, which already goes through
    /// the app's proxy setting, so there is nothing to apply here.
    pub fn set_http_proxy(&self, _proxy: Option<&str>) -> Result<(), Error> {
        Ok(())
    }

    /// Per-track loudness gain in dB. Positive gains get the engine's peak limiter, the same
    /// "only touch what would clip" rule as the desktop `alimiter`.
    pub fn set_gain(&self, gain_db: Option<f64>) -> Result<(), Error> {
        send("player.gain", json!({ "db": gain_db.unwrap_or(0.0) }));
        Ok(())
    }

    pub fn set_speed(&self, speed: f64) -> Result<(), Error> {
        send("player.speed", json!({ "speed": speed.clamp(0.25, 2.0) }));
        Ok(())
    }

    pub fn set_pitch(&self, semitones: i32) -> Result<(), Error> {
        // AVPlayer can change the tempo on its own but not the pitch.
        #[cfg(target_os = "ios")]
        if semitones != 0 {
            return Err(Error::NoPitchFilter);
        }
        let factor = 2f64.powf(semitones.clamp(-12, 12) as f64 / 12.0);
        send("player.pitch", json!({ "factor": factor }));
        Ok(())
    }

    /// ExoPlayer's silence skipping. Android only (Metrolist's "Skip silence").
    pub fn set_skip_silence(&self, on: bool) {
        send("player.skipSilence", json!({ "on": on }));
    }

    pub fn current_path(&self) -> Option<String> {
        self.mirror.path.lock().unwrap().clone()
    }

    // The music video plays in the webview on Android (the macOS path), so the native video
    // surface API is all no-ops.
    pub fn set_video_for(&self, _audio_url: &str, _video_url: &str) {}

    pub fn set_video_visible(&self, _on: bool) {}

    pub fn set_video_window(&self, _wid: i64) -> Result<(), Error> {
        Ok(())
    }

    pub fn video_output_up(&self) -> bool {
        false
    }

    pub fn video_thumbnail(&self, _max_width: usize, _unless_at: Option<f64>) -> Option<Thumbnail> {
        None
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        send("player.stop", Value::Null);
    }
}

/// Slider percent → linear amplitude for ExoPlayer, matching the desktop curve exactly: mpv's
/// `volume` is `100·10^(−(1−s/100)^1.5)` and mpv applies `(v/100)³` to it, so the amplitude is
/// `10^(−3·(1−s/100)^1.5)` (−60 dB at the bottom of the slider, 0 dB at the top). 0 is a mute.
fn perceptual_gain(percent: i64) -> f64 {
    if percent <= 0 {
        return 0.0;
    }
    10f64.powf(-3.0 * (1.0 - percent.min(100) as f64 / 100.0).powf(1.5))
}

/// One event from `PlayerEngine.kt`. Every event carries the engine's idle flag and the path it
/// is playing, so the mirror is current *before* the app sees an end or a failure and asks
/// [`Player::is_idle`].
fn on_event(m: &Mirror, ev: &Value) {
    if let Some(idle) = ev.get("idle").and_then(Value::as_bool) {
        m.idle.store(idle, Ordering::SeqCst);
    }
    if let Some(path) = ev.get("path") {
        *m.path.lock().unwrap() = path.as_str().map(str::to_owned);
    }
    let num = |k: &str| ev.get(k).and_then(Value::as_f64);
    let text = |k: &str| ev.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    let out = match ev.get("ev").and_then(Value::as_str).unwrap_or_default() {
        "position" => num("v").map(PlayerEvent::Position),
        "duration" => num("v").map(PlayerEvent::Duration),
        "playing" => ev.get("v").and_then(Value::as_bool).map(PlayerEvent::Playing),
        "paused" => ev.get("v").and_then(Value::as_bool).map(PlayerEvent::Paused),
        "ended" => Some(PlayerEvent::TrackEnded),
        "failed" => Some(PlayerEvent::TrackFailed(text("msg"))),
        "lookaheadFailed" => Some(PlayerEvent::LookaheadFailed(text("msg"))),
        "deviceLost" => Some(PlayerEvent::AudioDeviceLost),
        "error" => Some(PlayerEvent::Error(text("msg"))),
        _ => None,
    };
    if let Some(e) = out {
        let _ = m.tx.send(e);
    }
}

#[cfg(test)]
mod tests {
    use super::perceptual_gain;

    #[test]
    fn volume_curve_matches_desktop() {
        assert_eq!(perceptual_gain(0), 0.0);
        assert!((perceptual_gain(100) - 1.0).abs() < 1e-12);
        // 50% is −21 dB on desktop (60·(0.5)^1.5 ≈ 21.2).
        let db = 20.0 * perceptual_gain(50).log10();
        assert!((db + 21.21).abs() < 0.05, "{db}");
    }
}
