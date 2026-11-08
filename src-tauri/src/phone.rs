//! Phone-style extras: the sleep timer, and the `native_action` command the mobile UI uses to
//! reach the native side (share sheet, system equalizer, keep-screen-on).
//!
//! The sleep timer lives here rather than in the UI because a webview's timers are throttled or
//! frozen with the screen off, which is exactly when a sleep timer has to fire. It works the same
//! on desktop; only the Android UI exposes it for now.

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;

#[derive(Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SleepState {
    /// When playback pauses, in ms since the epoch.
    pub at: Option<u64>,
    /// Pause when the current track ends instead.
    pub end_of_track: bool,
}

struct Timer {
    state: SleepState,
    /// Bumped on every change, so a sleeping task for an old setting knows to do nothing.
    gen: u64,
}

static TIMER: Mutex<Timer> = Mutex::new(Timer { state: SleepState { at: None, end_of_track: false }, gen: 0 });

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn publish(app: &AppHandle, s: SleepState) {
    let _ = app.emit("sleep-timer", s);
}

/// Pause now, for the timer. Through `pause` like every other pause, so a fade is cut and the UI,
/// the media session and Listen Together all hear about it the usual way.
fn fire(app: &AppHandle) {
    if let Some(state) = app.try_state::<Arc<AppState>>() {
        let _ = state.player.pause();
    }
    tracing::info!("sleep timer: paused");
    publish(app, SleepState::default());
}

#[tauri::command]
pub fn get_sleep_timer() -> SleepState {
    TIMER.lock().unwrap().state
}

/// `minutes` starts a countdown; `end_of_track` waits for the current song to finish; neither
/// turns the timer off.
#[tauri::command]
pub fn set_sleep_timer(app: AppHandle, minutes: Option<f64>, end_of_track: bool) -> SleepState {
    let mins = minutes.filter(|m| m.is_finite() && *m > 0.0);
    let (s, gen) = {
        let mut t = TIMER.lock().unwrap();
        t.gen += 1;
        t.state = SleepState {
            at: mins.map(|m| now_ms() + (m * 60_000.0) as u64),
            end_of_track: end_of_track && mins.is_none(),
        };
        (t.state, t.gen)
    };
    if let Some(m) = mins {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs_f64(m * 60.0)).await;
            let due = {
                let mut t = TIMER.lock().unwrap();
                let due = t.gen == gen;
                if due {
                    t.state = SleepState::default();
                }
                due
            };
            if due {
                fire(&app);
            }
        });
    }
    publish(&app, s);
    s
}

/// Called on every track end. Returns whether the end-of-track timer was armed (and has now fired).
pub fn on_track_ended(app: &AppHandle) -> bool {
    let armed = {
        let mut t = TIMER.lock().unwrap();
        let armed = t.state.end_of_track;
        if armed {
            t.gen += 1;
            t.state = SleepState::default();
        }
        armed
    };
    if armed {
        fire(app);
    }
    armed
}

/// Phone-only actions from the mobile UI. Desktop has no share sheet or system equalizer to open,
/// so there it reports that instead of pretending.
#[tauri::command]
pub fn native_action(action: String, args: serde_json::Value) -> Result<(), String> {
    #[cfg(mobile)]
    {
        let cmd = match action.as_str() {
            "share" => "app.share",
            "equalizer" => "app.equalizer",
            "keepScreenOn" => "app.keepScreenOn",
            "statusBar" => "app.statusBar",
            "background" => "app.background",
            "audioPermission" => "app.audioPermission",
            "insets" => "app.insets",
            "haptic" => "app.haptic",
            other => return Err(format!("unknown action {other}")),
        };
        player::bridge::send(cmd, args);
        Ok(())
    }
    #[cfg(not(mobile))]
    {
        let _ = args;
        Err(format!("{action} is only available on a phone"))
    }
}

/// Forward the Android side's `app` events (permission answers, the equalizer's) to the UI as
/// `android-event`. Called once from setup.
#[cfg(mobile)]
pub fn forward_android_events(app: AppHandle) {
    player::bridge::on("app", move |ev| {
        if ev.get("ev").and_then(|v| v.as_str()) == Some("activityRecreated") {
            rebuild_main_window(app.clone());
            return;
        }
        let _ = app.emit("android-event", ev.clone());
    });
}

/// The main window is tied to the activity it was built for (tao's Android windows *are*
/// activities). When that activity is destroyed while the process keeps running, the next one
/// arrives with no window in it: build the configured one again, which tao hands to the newest
/// activity without a window. Off the JNI callback, which runs on the Android main thread that the
/// build has to wait on.
#[cfg(mobile)]
fn rebuild_main_window(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        if app.get_webview_window("main").is_some() {
            return;
        }
        let Some(config) = app.config().app.windows.first().cloned() else { return };
        match tauri::WebviewWindowBuilder::from_config(&app, &config).and_then(|b| b.build()) {
            Ok(_) => tracing::info!("rebuilt the main window for a new activity"),
            Err(e) => tracing::warn!(error = %e, "could not rebuild the main window"),
        }
    });
}

// ---- Discord sign-in (Android) ------------------------------------------------------------------
// Desktop shows presence through the local Discord client and has nothing to sign in to.

#[tauri::command]
pub fn discord_account(state: tauri::State<'_, Arc<AppState>>) -> Result<serde_json::Value, String> {
    #[cfg(mobile)]
    {
        serde_json::to_value(crate::discord_net::account(&state.db)).map_err(|e| e.to_string())
    }
    #[cfg(not(mobile))]
    {
        let _ = state;
        Ok(serde_json::Value::Null)
    }
}

#[tauri::command]
pub fn discord_set_app_id(app_id: String) -> Result<(), String> {
    #[cfg(mobile)]
    {
        crate::discord_net::set_app_id(&app_id)
    }
    #[cfg(not(mobile))]
    {
        let _ = app_id;
        Err("only on Android".into())
    }
}

#[tauri::command]
pub fn discord_login() -> Result<(), String> {
    #[cfg(mobile)]
    {
        crate::discord_net::login()
    }
    #[cfg(not(mobile))]
    {
        Err("only on Android".into())
    }
}

#[tauri::command]
pub fn discord_logout() {
    #[cfg(mobile)]
    crate::discord_net::logout();
}
