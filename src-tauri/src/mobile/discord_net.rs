//! Discord Rich Presence on Android, the way Metrolist does it: there is no Discord client on the
//! phone to hand a card to, so the app signs in *as the user* with Discord's official OAuth2 (PKCE,
//! Social SDK scope `sdk.social_layer_presence`) and pushes `PRESENCE_UPDATE` over its own Gateway
//! connection.
//!
//! Everything about *what* the card shows stays in `discord.rs`, shared with desktop: this module
//! is only the transport. [`DiscordIpcClient`] has the methods `discord.rs` calls on the desktop IPC
//! client (`connect`, `set_activity`, `clear_activity`, `recv`, `close`), so the presence thread,
//! its rate limits and its settings run unchanged.
//!
//! The OAuth app is the user's own (an Application ID they paste into Settings ▸ Discord): the
//! Limusic app id the desktop build uses has no redirect URI or public-client flow set up for this,
//! and it isn't ours to change. Tokens live in the `discord_oauth` setting, which never crosses into
//! the webview (it is not in `UI_SETTINGS`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use base64::Engine;
use discord_rich_presence::activity;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::db::{now_secs, Db};
use crate::state::AppState;

const AUTHORIZE: &str = "https://discord.com/oauth2/authorize";
const TOKEN_URL: &str = "https://discord.com/api/v10/oauth2/token";
const SCOPES: &str = "openid sdk.social_layer_presence";
/// Must be registered, exactly, under OAuth2 ▸ Redirects of the user's Discord app. The Android
/// side catches it (`DiscordOAuthActivity`, manifest intent filter).
pub const REDIRECT_URI: &str = "limusicdiscord://oauth2/callback";
const GATEWAY: &str = "wss://gateway.discord.gg/?v=10&encoding=json";
const EXTERNAL_ASSETS: &str = "https://discord.com/api/v9/applications/{id}/external-assets";
const SETTING: &str = "discord_oauth";
/// Refresh this long before the access token (7 days) runs out.
const REFRESH_MARGIN: i64 = 3600;
const USER_AGENT: &str = "Discord-Android/314013;RNA";

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
struct Stored {
    app_id: String,
    access: String,
    refresh: String,
    expires_at: i64,
    user: Option<String>,
    avatar: Option<String>,
}

/// What the settings tab shows. Never the tokens.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub app_id: String,
    pub signed_in: bool,
    pub user: Option<String>,
    pub avatar: Option<String>,
    pub redirect_uri: &'static str,
}

struct Globals {
    app: AppHandle,
    db: Arc<Db>,
    /// The sign-in in flight: (state, PKCE verifier).
    pending: Mutex<Option<(String, String)>>,
    /// Discord refused the token outright: stop re-identifying every few seconds until the user
    /// signs in again (identifies are rate-limited per day).
    auth_failed: AtomicBool,
    assets: Mutex<HashMap<String, String>>,
}

static G: OnceLock<Globals> = OnceLock::new();

fn g() -> Result<&'static Globals, NetError> {
    G.get().ok_or_else(|| NetError("discord not initialised".into()))
}

fn load(db: &Db) -> Stored {
    db.get_setting(SETTING).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(db: &Db, s: &Stored) {
    if let Ok(json) = serde_json::to_string(s) {
        db.set_setting(SETTING, &json);
    }
}

pub fn account(db: &Db) -> Account {
    let s = load(db);
    Account {
        app_id: s.app_id,
        signed_in: !s.access.is_empty(),
        user: s.user,
        avatar: s.avatar,
        redirect_uri: REDIRECT_URI,
    }
}

fn changed() {
    if let Some(g) = G.get() {
        let _ = g.app.emit("discord-account", account(&g.db));
    }
}

/// Reconnect presence from scratch: after a sign-in it starts at once instead of waiting out the
/// presence thread's connect backoff, and after a sign-out the old socket goes away.
fn restart_presence() {
    let Some(g) = G.get() else { return };
    let Some(state) = g.app.try_state::<Arc<AppState>>() else { return };
    state.set_discord_enabled(false);
    if g.db.get_setting("discord_rpc").as_deref() == Some("true") {
        state.set_discord_enabled(true);
    }
}

pub fn init(app: AppHandle, db: Arc<Db>) {
    let _ = G.set(Globals {
        app,
        db,
        pending: Mutex::new(None),
        auth_failed: AtomicBool::new(false),
        assets: Mutex::new(HashMap::new()),
    });
    player::bridge::on("discord", |ev| {
        let ev = ev.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = on_callback(&ev).await {
                tracing::warn!(error = %e, "discord sign-in failed");
                if let Some(g) = G.get() {
                    let _ = g.app.emit("discord-error", e.0);
                }
            }
        });
    });
}

// ---- sign-in ---------------------------------------------------------------------------------

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn set_app_id(app_id: &str) -> Result<(), String> {
    let g = g().map_err(|e| e.0)?;
    let id = app_id.trim();
    if !id.is_empty() && !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("An Application ID is only digits".into());
    }
    let mut s = load(&g.db);
    if s.app_id != id {
        // Tokens belong to the app that issued them.
        s = Stored { app_id: id.to_owned(), ..Default::default() };
        save(&g.db, &s);
        restart_presence();
    }
    changed();
    Ok(())
}

/// Open Discord's own consent page. The answer comes back through the redirect, to
/// [`on_callback`].
pub fn login() -> Result<(), String> {
    let g = g().map_err(|e| e.0)?;
    let s = load(&g.db);
    if s.app_id.is_empty() {
        return Err("Paste your Discord Application ID first".into());
    }
    let mut seed = [0u8; 48];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut seed);
    let verifier = b64(&seed);
    let challenge = b64(&Sha256::digest(verifier.as_bytes()));
    let state = b64(&rand::random::<[u8; 16]>());
    *g.pending.lock().unwrap() = Some((state.clone(), verifier));
    let url = format!(
        "{AUTHORIZE}?client_id={}&response_type=code&redirect_uri={}&scope={}&state={}&code_challenge_method=S256&code_challenge={}",
        s.app_id,
        urlencoding::encode(REDIRECT_URI),
        urlencoding::encode(SCOPES),
        state,
        challenge,
    );
    player::bridge::send("app.openUrl", json!({ "url": url }));
    Ok(())
}

pub fn logout() {
    let Ok(g) = g() else { return };
    let s = load(&g.db);
    save(&g.db, &Stored { app_id: s.app_id, ..Default::default() });
    g.auth_failed.store(false, Ordering::SeqCst);
    restart_presence();
    changed();
}

async fn on_callback(ev: &Value) -> Result<(), NetError> {
    let g = g()?;
    let field = |k: &str| ev.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    if !field("error").is_empty() {
        *g.pending.lock().unwrap() = None;
        return Err(NetError(format!("Discord said: {}", field("error"))));
    }
    let (state, verifier) = g
        .pending
        .lock()
        .unwrap()
        .take()
        .ok_or_else(|| NetError("no sign-in was in progress".into()))?;
    if field("state") != state {
        return Err(NetError("the sign-in answer did not match (state)".into()));
    }
    let app_id = load(&g.db).app_id;
    let tokens = token_request(&[
        ("client_id", app_id.as_str()),
        ("grant_type", "authorization_code"),
        ("code", &field("code")),
        ("redirect_uri", REDIRECT_URI),
        ("code_verifier", &verifier),
    ])
    .await?;
    let mut s = load(&g.db);
    s.access = tokens.0;
    s.refresh = tokens.1;
    s.expires_at = now_secs() + tokens.2;
    save(&g.db, &s);
    g.auth_failed.store(false, Ordering::SeqCst);
    tracing::info!("discord: signed in");
    changed();
    restart_presence();
    Ok(())
}

/// POST to the token endpoint as a public client (no secret: PKCE stands in for it).
/// Returns (access, refresh, expires_in).
async fn token_request(params: &[(&str, &str)]) -> Result<(String, String, i64), NetError> {
    let body: String = params
        .iter()
        .map(|(k, v)| format!("{k}={}", urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    let resp = crate::http::client()
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .timeout(Duration::from_secs(15))
        .body(body)
        .send()
        .await
        .map_err(|e| NetError(format!("couldn't reach Discord: {e}")))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let err = v.get("error_description").or(v.get("error")).and_then(Value::as_str);
        return Err(NetError(format!("Discord refused the sign-in ({status}): {}", err.unwrap_or("?"))));
    }
    let access = v.get("access_token").and_then(Value::as_str).unwrap_or_default().to_owned();
    if access.is_empty() {
        return Err(NetError("Discord sent no access token".into()));
    }
    Ok((
        access,
        v.get("refresh_token").and_then(Value::as_str).unwrap_or_default().to_owned(),
        v.get("expires_in").and_then(Value::as_i64).unwrap_or(604_800),
    ))
}

/// The access token, refreshed first when it is about to run out.
async fn fresh_token() -> Result<(String, String), NetError> {
    let g = g()?;
    let mut s = load(&g.db);
    if s.access.is_empty() || s.app_id.is_empty() {
        return Err(NetError("not signed in to Discord".into()));
    }
    if s.expires_at - now_secs() < REFRESH_MARGIN && !s.refresh.is_empty() {
        match token_request(&[
            ("client_id", s.app_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", s.refresh.as_str()),
        ])
        .await
        {
            Ok((access, refresh, expires_in)) => {
                s.access = access;
                if !refresh.is_empty() {
                    s.refresh = refresh;
                }
                s.expires_at = now_secs() + expires_in;
                save(&g.db, &s);
                tracing::info!("discord: token refreshed");
            }
            Err(e) => tracing::warn!(error = %e, "discord: token refresh failed"),
        }
    }
    Ok((s.access, s.app_id))
}

// ---- the Gateway client ----------------------------------------------------------------------

#[derive(Debug)]
pub struct NetError(String);

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<E: std::error::Error> From<E> for NetError {
    fn from(e: E) -> Self {
        NetError(e.to_string())
    }
}

struct Conn {
    out: mpsc::UnboundedSender<String>,
    alive: Arc<AtomicBool>,
}

/// Stand-in for `discord_rich_presence::DiscordIpcClient`, over the Gateway. Blocking, like the
/// original, because `discord.rs` drives it from its own thread.
pub struct DiscordIpcClient {
    conn: Option<Conn>,
}

impl DiscordIpcClient {
    pub fn new<T: AsRef<str>>(_desktop_app_id: T) -> Self {
        DiscordIpcClient { conn: None }
    }

    pub fn connect(&mut self) -> Result<(), NetError> {
        let conn = tauri::async_runtime::block_on(open())?;
        self.conn = Some(conn);
        Ok(())
    }

    pub fn set_activity(&mut self, act: activity::Activity) -> Result<(), NetError> {
        let raw = serde_json::to_value(&act)?;
        let activity = tauri::async_runtime::block_on(to_gateway(raw));
        self.send(presence_frame(Some(activity)))
    }

    pub fn clear_activity(&mut self) -> Result<(), NetError> {
        self.send(presence_frame(None))
    }

    /// The IPC socket answers every command; the Gateway does not answer a presence update. A live
    /// socket is the whole answer.
    pub fn recv(&mut self) -> Result<(u32, Value), NetError> {
        match &self.conn {
            Some(c) if c.alive.load(Ordering::SeqCst) => Ok((1, json!({}))),
            _ => Err(NetError("gateway closed".into())),
        }
    }

    pub fn close(&mut self) -> Result<(), NetError> {
        self.conn = None; // dropping the sender closes the socket
        Ok(())
    }

    fn send(&mut self, frame: String) -> Result<(), NetError> {
        let c = self.conn.as_ref().ok_or_else(|| NetError("not connected".into()))?;
        if !c.alive.load(Ordering::SeqCst) || c.out.send(frame).is_err() {
            self.conn = None;
            return Err(NetError("gateway closed".into()));
        }
        Ok(())
    }
}

fn presence_frame(activity: Option<Value>) -> String {
    json!({
        "op": 3,
        "d": {
            "since": 0,
            "activities": activity.map(|a| vec![a]).unwrap_or_default(),
            "status": "online",
            "afk": false,
        }
    })
    .to_string()
}

fn super_properties() -> String {
    let props = json!({
        "os": "Android",
        "browser": "Discord Android",
        "device": "limusic",
        "client_version": "314.13 - Stable",
        "release_channel": "googleRelease",
        "client_build_number": 314013,
    });
    base64::engine::general_purpose::STANDARD.encode(props.to_string())
}

/// The IPC activity, reshaped for a Gateway `PRESENCE_UPDATE`: images become Discord-proxied
/// `mp:` assets, and buttons split into labels plus `metadata.button_urls`.
async fn to_gateway(mut a: Value) -> Value {
    if a.get("name").is_none() {
        a["name"] = json!("Limusic");
    }
    if let Some(assets) = a.get_mut("assets").and_then(Value::as_object_mut) {
        for key in ["large_image", "small_image"] {
            let Some(url) = assets.get(key).and_then(Value::as_str).map(str::to_owned) else {
                continue;
            };
            match proxied_image(&url).await {
                Some(mp) => {
                    assets.insert(key.into(), json!(mp));
                }
                None => {
                    assets.remove(key);
                }
            }
        }
    }
    if let Some(buttons) = a.get("buttons").and_then(Value::as_array).cloned() {
        let labels: Vec<Value> = buttons.iter().filter_map(|b| b.get("label").cloned()).collect();
        let urls: Vec<Value> = buttons.iter().filter_map(|b| b.get("url").cloned()).collect();
        a["buttons"] = json!(labels);
        a["metadata"] = json!({ "button_urls": urls });
    }
    a
}

/// `https://…` → `mp:external/…`. The Gateway only shows images Discord has proxied, and this
/// endpoint is what proxies them. Cached, since the same cover comes back on every refresh.
async fn proxied_image(url: &str) -> Option<String> {
    if url.starts_with("mp:") {
        return Some(url.to_owned());
    }
    let g = G.get()?;
    if let Some(hit) = g.assets.lock().unwrap().get(url) {
        return Some(hit.clone());
    }
    let (token, app_id) = fresh_token().await.ok()?;
    let resp = crate::http::client()
        .post(EXTERNAL_ASSETS.replace("{id}", &app_id))
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", USER_AGENT)
        .header("X-Super-Properties", super_properties())
        .header("Content-Type", "application/json")
        .timeout(Duration::from_secs(10))
        .body(json!({ "urls": [url] }).to_string())
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        tracing::debug!(status = %resp.status(), "discord: external asset refused");
        return None;
    }
    let v: Value = resp.json().await.ok()?;
    let path = v.get(0)?.get("external_asset_path")?.as_str()?;
    let mp = format!("mp:{path}");
    let mut cache = g.assets.lock().unwrap();
    if cache.len() > 128 {
        cache.clear();
    }
    cache.insert(url.to_owned(), mp.clone());
    Some(mp)
}

async fn open() -> Result<Conn, NetError> {
    let g = g()?;
    if g.auth_failed.load(Ordering::SeqCst) {
        return Err(NetError("Discord rejected the sign-in; sign in again".into()));
    }
    let (token, app_id) = fresh_token().await?;
    let (ws, _) = tokio::time::timeout(Duration::from_secs(15), tokio_tungstenite::connect_async(GATEWAY))
        .await
        .map_err(|_| NetError("gateway connect timed out".into()))??;
    let (mut sink, mut stream) = ws.split();

    let mut seq: Option<u64> = None;
    let mut interval_ms = 41_250u64;
    let identify = json!({
        "op": 2,
        "d": {
            "token": format!("Bearer {token}"),
            "intents": 0,
            "properties": { "os": "android", "browser": "Discord Android", "device": app_id },
            "compress": false,
        }
    })
    .to_string();
    let mut identified = false;

    // Hello, then Identify, then READY.
    let ready = loop {
        let next = tokio::time::timeout(Duration::from_secs(20), stream.next())
            .await
            .map_err(|_| NetError("gateway handshake timed out".into()))?;
        let text = match next {
            Some(Ok(Message::Text(t))) => t,
            Some(Ok(Message::Close(frame))) => {
                let code = frame.as_ref().map(|f| u16::from(f.code)).unwrap_or(0);
                if code == 4004 {
                    g.auth_failed.store(true, Ordering::SeqCst);
                    changed();
                }
                return Err(NetError(format!("gateway closed during handshake ({code})")));
            }
            Some(Ok(_)) => continue,
            Some(Err(e)) => return Err(e.into()),
            None => return Err(NetError("gateway hung up".into())),
        };
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if let Some(s) = v.get("s").and_then(Value::as_u64) {
            seq = Some(s);
        }
        match v.get("op").and_then(Value::as_u64) {
            Some(10) => {
                interval_ms = v.pointer("/d/heartbeat_interval").and_then(Value::as_u64).unwrap_or(interval_ms);
                if !identified {
                    sink.send(Message::Text(identify.clone())).await?;
                    identified = true;
                }
            }
            Some(1) => {
                sink.send(Message::Text(json!({ "op": 1, "d": seq }).to_string())).await?;
            }
            Some(9) => return Err(NetError("gateway refused the session".into())),
            Some(0) if v.get("t").and_then(Value::as_str) == Some("READY") => break v,
            _ => {}
        }
    };

    // Who signed in, for the settings tab.
    if let Some(u) = ready.pointer("/d/user") {
        let mut s = load(&g.db);
        s.user = u
            .get("global_name")
            .and_then(Value::as_str)
            .or_else(|| u.get("username").and_then(Value::as_str))
            .map(str::to_owned);
        s.avatar = match (u.get("id").and_then(Value::as_str), u.get("avatar").and_then(Value::as_str)) {
            (Some(id), Some(hash)) => Some(format!("https://cdn.discordapp.com/avatars/{id}/{hash}.png?size=128")),
            _ => None,
        };
        save(&g.db, &s);
        changed();
    }
    tracing::info!("discord gateway ready");

    let (out, mut out_rx) = mpsc::unbounded_channel::<String>();
    let alive = Arc::new(AtomicBool::new(true));
    let alive2 = alive.clone();
    tauri::async_runtime::spawn(async move {
        let mut hb = tokio::time::interval(Duration::from_millis(interval_ms.max(1000)));
        hb.tick().await; // the first tick is immediate
        let mut acked = true;
        loop {
            tokio::select! {
                _ = hb.tick() => {
                    if !acked {
                        tracing::debug!("discord gateway: missed heartbeat ack, dropping");
                        break;
                    }
                    acked = false;
                    let beat = json!({ "op": 1, "d": seq }).to_string();
                    if sink.send(Message::Text(beat)).await.is_err() { break; }
                }
                frame = out_rx.recv() => match frame {
                    Some(text) => if sink.send(Message::Text(text)).await.is_err() { break },
                    None => {
                        let _ = sink.close().await;
                        break;
                    }
                },
                msg = stream.next() => match msg {
                    Some(Ok(Message::Text(t))) => {
                        let v: Value = serde_json::from_str(&t).unwrap_or(Value::Null);
                        if let Some(s) = v.get("s").and_then(Value::as_u64) { seq = Some(s); }
                        match v.get("op").and_then(Value::as_u64) {
                            Some(11) => acked = true,
                            Some(1) => {
                                let beat = json!({ "op": 1, "d": seq }).to_string();
                                if sink.send(Message::Text(beat)).await.is_err() { break; }
                            }
                            Some(7) | Some(9) => break,
                            _ => {}
                        }
                    }
                    Some(Ok(Message::Close(frame))) => {
                        let code = frame.as_ref().map(|f| u16::from(f.code)).unwrap_or(0);
                        tracing::info!(code, "discord gateway closed");
                        break;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => break,
                },
            }
        }
        alive2.store(false, Ordering::SeqCst);
    });
    Ok(Conn { out, alive })
}
