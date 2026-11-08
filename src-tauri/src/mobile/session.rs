//! Android sign-in: the desktop login window becomes a full-screen native sign-in screen
//! (`LoginActivity.kt`), and the hidden session-refresh window a hidden `WebView`. Both run on the
//! app's shared Android `CookieManager`, which is persistent like the desktop login webview, so a
//! re-login is one tap and a rejected session can be re-minted without the user (#165 / KI-2).
//!
//! The Kotlin side reads the youtube.com jar with `CookieManager.getCookie`, which answers with
//! the same `name=value; …` header [`youtube_cookie_header`] builds on desktop, HttpOnly cookies
//! included.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

use crate::state::{AppState, SignInOutcome};
use player::bridge;

const LOGIN_URL: &str =
    "https://accounts.google.com/ServiceLogin?service=youtube&continue=https://music.youtube.com/";
const ADD_ACCOUNT_URL: &str =
    "https://accounts.google.com/AddSession?service=youtube&continue=https://music.youtube.com/";
const REFRESH_URL: &str = "https://music.youtube.com/";
const REFRESH_TIMEOUT: Duration = Duration::from_secs(45);
const REFRESH_COOLDOWN: Duration = Duration::from_secs(5 * 60);

/// What the Kotlin side reported. Only one flow of each kind runs at a time, so a single slot per
/// kind is enough.
struct Slots {
    login: Mutex<Option<mpsc::UnboundedSender<Value>>>,
    refresh: Mutex<Option<mpsc::UnboundedSender<String>>>,
}

fn slots() -> &'static Slots {
    static S: OnceLock<Slots> = OnceLock::new();
    S.get_or_init(|| {
        bridge::on("login", |ev| {
            let s = slots();
            match ev.get("ev").and_then(Value::as_str) {
                Some("refresh") => {
                    let cookie = ev.get("cookie").and_then(Value::as_str).unwrap_or_default();
                    if let Some(tx) = s.refresh.lock().unwrap().as_ref() {
                        let _ = tx.send(cookie.to_owned());
                    }
                }
                _ => {
                    if let Some(tx) = s.login.lock().unwrap().as_ref() {
                        let _ = tx.send(ev.clone());
                    }
                }
            }
        });
        Slots { login: Mutex::new(None), refresh: Mutex::new(None) }
    })
}

pub fn open_login(app: AppHandle, state: Arc<AppState>, add_account: bool) {
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    *slots().login.lock().unwrap() = Some(tx);
    let url = if add_account { ADD_ACCOUNT_URL } else { LOGIN_URL };
    bridge::send("login.open", json!({ "url": url, "clear": add_account }));

    tauri::async_runtime::spawn(async move {
        // One report per page load on music.youtube.com, or a final "closed" when the user backs
        // out. The jar can lag the page by a moment, so a load without SAPISID is not the end.
        while let Some(ev) = rx.recv().await {
            match ev.get("ev").and_then(Value::as_str) {
                Some("cookies") => {
                    let cookie = ev.get("cookie").and_then(Value::as_str).unwrap_or_default();
                    if innertube::cookie_sapisid(cookie).is_none() {
                        continue;
                    }
                    match state.sign_in(cookie.to_owned(), None).await {
                        Ok(SignInOutcome::Complete) => {
                            let _ = app.emit("login-done", ());
                        }
                        Ok(SignInOutcome::SelectionRequired) => {}
                        Err(e) => {
                            let _ = app.emit("login-error", e);
                        }
                    }
                    bridge::send("login.close", Value::Null);
                    break;
                }
                Some("error") => {
                    let msg = ev.get("msg").and_then(Value::as_str).unwrap_or("Sign-in failed");
                    let _ = app.emit("login-error", msg.to_owned());
                    break;
                }
                _ => break, // closed by the user
            }
        }
        *slots().login.lock().unwrap() = None;
    });
}

pub async fn refresh_session(_app: AppHandle, state: Arc<AppState>) {
    if !state.it.is_logged_in() || !claim_refresh() {
        return;
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    *slots().refresh.lock().unwrap() = Some(tx);
    bridge::send("login.refresh", json!({ "url": REFRESH_URL }));

    let deadline = Instant::now() + REFRESH_TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok(Some(cookie)) = tokio::time::timeout(left, rx.recv()).await else {
            tracing::debug!("session refresh: the webview had nothing fresher to offer");
            break;
        };
        let current = state.it.cookie().unwrap_or_default();
        let Some(account) = innertube::cookie_sapisid(&current) else { break };
        if innertube::cookie_sapisid(&cookie) == Some(account) && !same_jar(&cookie, &current) {
            match state.sign_in(cookie, Some(account)).await {
                Ok(_) => tracing::info!("re-minted the login session from the login webview"),
                Err(error) => tracing::warn!(%error, "could not re-mint the login session"),
            }
            break;
        }
    }
    *slots().refresh.lock().unwrap() = None;
    bridge::send("login.refreshDone", Value::Null);
}

fn same_jar(a: &str, b: &str) -> bool {
    let pairs = |s: &str| {
        let mut v: Vec<String> = s.split(';').map(|kv| kv.trim().to_owned()).collect();
        v.sort();
        v
    };
    pairs(a) == pairs(b)
}

fn claim_refresh() -> bool {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap();
    if last.is_some_and(|t| t.elapsed() < REFRESH_COOLDOWN) {
        return false;
    }
    *last = Some(Instant::now());
    true
}
