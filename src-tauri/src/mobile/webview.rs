//! Android: the hidden JS webview behind [`Bridge`], as a plain `android.webkit.WebView` the Kotlin
//! side keeps off screen (`HiddenWebViews.kt`). Tauri allows one webview per app on Android, and
//! that one is the UI, so the cipher harness cannot be a second Tauri window the way it is on
//! desktop. The API is the desktop one, so `cipher/mod.rs` is unchanged.
//!
//! `eval_json` round-trips through `evaluateJavascript`, whose callback hands back the JSON
//! encoding of the expression's value: the same contract as wry's `eval_with_callback`.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex as StdMutex;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::AppHandle;
use tokio::sync::oneshot;

use player::bridge;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("webview '{0}' does not exist")]
    Gone(String),
    #[error("webview eval failed: {0}")]
    Eval(String),
    #[error("timed out after {0:?}")]
    Timeout(Duration),
    #[error("webview reported a JS error: {0}")]
    BadWebview(String),
    #[allow(dead_code)]
    #[error("build failed: {0}")]
    Build(String),
}

#[derive(Clone)]
pub struct Bridge {
    label: String,
}

/// Unused on Android (the harness is loaded with `loadDataWithBaseURL`), kept so lib.rs's scheme
/// registration reads the same on both.
pub const SCHEME: &str = "limusicharness";

pub const HARNESS_HTML: &str = "<!doctype html><html><head><meta charset=utf-8></head><body>\
<script>\
window.__jserr=null;window.__slots={};window._yt_player=window._yt_player||{};\
window.addEventListener('error',function(e){window.__jserr=String((e&&e.message)||e);});\
window.onunhandledrejection=function(e){window.__jserr=String((e.reason&&e.reason.message)||e.reason);};\
window.__harness=1;\
</script></body></html>";

struct Shared {
    live: StdMutex<HashSet<String>>,
    pending: StdMutex<HashMap<u64, oneshot::Sender<String>>>,
}

fn shared() -> &'static Shared {
    static S: OnceLock<Shared> = OnceLock::new();
    S.get_or_init(|| {
        bridge::on("webview", |ev| {
            let s = shared();
            match ev.get("ev").and_then(Value::as_str) {
                Some("result") => {
                    let Some(id) = ev.get("id").and_then(Value::as_u64) else { return };
                    let raw = ev.get("json").and_then(Value::as_str).unwrap_or("null").to_owned();
                    if let Some(tx) = s.pending.lock().unwrap().remove(&id) {
                        let _ = tx.send(raw);
                    }
                }
                Some("gone") => {
                    if let Some(label) = ev.get("label").and_then(Value::as_str) {
                        s.live.lock().unwrap().remove(label);
                    }
                }
                _ => {}
            }
        });
        Shared { live: Default::default(), pending: Default::default() }
    })
}

impl Bridge {
    pub async fn create(_app: &AppHandle, label: &str) -> Result<Bridge, Error> {
        let s = shared();
        bridge::send("webview.create", json!({ "label": label, "html": HARNESS_HTML }));
        s.live.lock().unwrap().insert(label.to_owned());
        let bridge = Bridge { label: label.to_owned() };

        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let ready =
                bridge.eval_json("window.__harness===1".into(), Duration::from_millis(800)).await;
            if matches!(ready, Ok(Value::Bool(true))) {
                tracing::info!(label, "webview bridge OK — harness loaded, eval round-trips");
                return Ok(bridge);
            }
            if Instant::now() >= deadline {
                tracing::error!(label, "hidden webview never became ready");
                let _ = bridge.destroy();
                return Err(Error::Timeout(Duration::from_secs(12)));
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }

    pub fn exists(&self) -> bool {
        shared().live.lock().unwrap().contains(&self.label)
    }

    pub fn eval(&self, js: &str) -> Result<(), Error> {
        if !self.exists() {
            return Err(Error::Gone(self.label.clone()));
        }
        bridge::send("webview.eval", json!({ "label": self.label, "js": js }));
        Ok(())
    }

    pub async fn eval_json(&self, js: String, timeout: Duration) -> Result<Value, Error> {
        if !self.exists() {
            return Err(Error::Gone(self.label.clone()));
        }
        let id = next_call_id();
        let (tx, rx) = oneshot::channel();
        shared().pending.lock().unwrap().insert(id, tx);
        bridge::send("webview.evalJson", json!({ "label": self.label, "js": js, "id": id }));
        let raw = match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(raw)) => raw,
            Ok(Err(_)) => return Err(Error::Eval("callback dropped".into())),
            Err(_) => {
                shared().pending.lock().unwrap().remove(&id);
                return Err(Error::Timeout(timeout));
            }
        };
        if raw.is_empty() || raw == "null" {
            return Ok(Value::Null);
        }
        serde_json::from_str(&raw).map_err(|e| Error::Eval(format!("bad json {raw:?}: {e}")))
    }

    async fn poll_json(&self, expr: &str, timeout: Duration) -> Result<Value, Error> {
        let deadline = Instant::now() + timeout;
        let step = Duration::from_millis(100);
        loop {
            match self.eval_json(format!("({expr})"), Duration::from_secs(3)).await {
                Ok(v) if !v.is_null() => return Ok(v),
                Ok(_) => {}
                Err(Error::Gone(l)) => return Err(Error::Gone(l)),
                Err(_) => {}
            }
            if let Ok(Some(msg)) = self
                .eval_json("window.__jserr||null".into(), Duration::from_secs(3))
                .await
                .map(|v| v.as_str().map(str::to_owned))
            {
                return Err(Error::BadWebview(msg));
            }
            if Instant::now() >= deadline {
                return Err(Error::Timeout(timeout));
            }
            tokio::time::sleep(step).await;
        }
    }

    pub async fn call_async(&self, expr: &str, timeout: Duration) -> Result<Value, Error> {
        let id = next_call_id();
        let kick = format!(
            "(async()=>{{try{{window.__slots['{id}']={{ok:1,v:await ({expr})}};}}\
             catch(e){{window.__slots['{id}']={{ok:0,e:String((e&&e.message)||e)}};}}}})();"
        );
        self.eval(&kick)?;
        let res = self.poll_json(&format!("window.__slots['{id}']||null"), timeout).await;
        let _ = self.eval(&format!("delete window.__slots['{id}'];"));
        let slot = res?;
        if slot.get("ok").and_then(Value::as_i64) == Some(1) {
            Ok(slot.get("v").cloned().unwrap_or(Value::Null))
        } else {
            Err(Error::Eval(
                slot.get("e").and_then(Value::as_str).unwrap_or("async call failed").to_owned(),
            ))
        }
    }

    pub fn destroy(&self) -> Result<(), Error> {
        shared().live.lock().unwrap().remove(&self.label);
        bridge::send("webview.destroy", json!({ "label": self.label }));
        Ok(())
    }
}

fn next_call_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

#[allow(dead_code)]
pub(crate) async fn destroy_and_wait(_app: &AppHandle, label: &str) {
    shared().live.lock().unwrap().remove(label);
    bridge::send("webview.destroy", json!({ "label": label }));
}
