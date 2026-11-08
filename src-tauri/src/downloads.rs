//! Offline downloads: songs saved to the device so they play with no connection, the way YouTube
//! Music, Spotify and Metrolist do it on a phone.
//!
//! A download is the same stream the player would get. It is resolved through the usual client
//! chain, pulled through the loopback audio proxy (so googlevideo serves it in bounded ranges at
//! full speed), and written to `<app data>/downloads/<videoId>.<ext>`. From then on
//! [`AppState::resolve`] answers that videoId with the file, so the queue, the UI, scrobbling and
//! Listen Together all keep treating it as the YouTube track it is: only the bytes are local.
//!
//! What is downloaded lives in the `downloads` setting as `videoId → entry`, with the song row the
//! Downloads page lists. One download runs at a time, in the order they were asked for.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use innertube::SongItem;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncWriteExt;

use crate::db::{now_secs, Db};
use crate::state::AppState;

const SETTING: &str = "downloads";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub song: SongItem,
    pub file: String,
    pub bytes: u64,
    pub added_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loudness_db: Option<f64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// Finished downloads, newest first.
    pub done: Vec<Entry>,
    /// Waiting or in flight, in order; the first one is the one downloading.
    pub queued: Vec<SongItem>,
    /// Bytes so far / total of the one in flight.
    pub progress: Option<(u64, u64)>,
}

struct Queue {
    waiting: VecDeque<SongItem>,
    running: bool,
    progress: Option<(u64, u64)>,
}

fn queue() -> &'static Mutex<Queue> {
    static Q: OnceLock<Mutex<Queue>> = OnceLock::new();
    Q.get_or_init(|| Mutex::new(Queue { waiting: VecDeque::new(), running: false, progress: None }))
}

pub fn dir(app: &AppHandle) -> PathBuf {
    app.path().app_data_dir().unwrap_or_else(|_| std::env::temp_dir()).join("downloads")
}

fn load(db: &Db) -> BTreeMap<String, Entry> {
    db.get_setting(SETTING).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(db: &Db, map: &BTreeMap<String, Entry>) {
    if let Ok(json) = serde_json::to_string(map) {
        db.set_setting(SETTING, &json);
    }
}

/// The file for a downloaded videoId, if it is still on disk. What `AppState::resolve` asks first.
pub fn local(db: &Db, video_id: &str) -> Option<Entry> {
    let e = load(db).remove(video_id)?;
    std::path::Path::new(&e.file).is_file().then_some(e)
}

pub fn status(db: &Db) -> Status {
    let mut done: Vec<Entry> = load(db).into_values().collect();
    done.sort_by(|a, b| b.added_at.cmp(&a.added_at));
    let q = queue().lock().unwrap();
    Status { done, queued: q.waiting.iter().cloned().collect(), progress: q.progress }
}

fn changed(app: &AppHandle, db: &Db) {
    let _ = app.emit("downloads-changed", status(db));
}

/// Queue songs for download. Already downloaded or already queued ones are skipped.
pub fn enqueue(state: Arc<AppState>, songs: Vec<SongItem>) {
    let have = load(&state.db);
    {
        let mut q = queue().lock().unwrap();
        for s in songs {
            if crate::local::is_local_song(&s.video_id)
                || have.contains_key(&s.video_id)
                || q.waiting.iter().any(|w| w.video_id == s.video_id)
            {
                continue;
            }
            q.waiting.push_back(s);
        }
        if q.running || q.waiting.is_empty() {
            drop(q);
            changed(&state.app, &state.db);
            return;
        }
        q.running = true;
    }
    changed(&state.app, &state.db);
    tauri::async_runtime::spawn(worker(state));
}

async fn worker(state: Arc<AppState>) {
    loop {
        let next = {
            let mut q = queue().lock().unwrap();
            match q.waiting.front().cloned() {
                Some(s) => {
                    q.progress = Some((0, 0));
                    s
                }
                None => {
                    q.running = false;
                    q.progress = None;
                    break;
                }
            }
        };
        let id = next.video_id.clone();
        match fetch(&state, &next).await {
            Ok(entry) => {
                let mut map = load(&state.db);
                map.insert(id.clone(), entry);
                save(&state.db, &map);
                tracing::info!(video_id = %id, "downloaded");
            }
            Err(e) => {
                tracing::warn!(video_id = %id, error = %e, "download failed");
                let _ = state.app.emit(
                    "download-failed",
                    serde_json::json!({ "videoId": id, "title": next.title, "error": e }),
                );
            }
        }
        {
            let mut q = queue().lock().unwrap();
            if q.waiting.front().is_some_and(|s| s.video_id == id) {
                q.waiting.pop_front();
            }
        }
        changed(&state.app, &state.db);
    }
    changed(&state.app, &state.db);
}

async fn fetch(state: &Arc<AppState>, song: &SongItem) -> Result<Entry, String> {
    let data = state.resolve_stream(&song.video_id).await?;
    if !data.stream_url.starts_with("http") {
        return Err("already a local file".into());
    }
    // Through the loopback proxy, which turns one long GET into the bounded ranges googlevideo
    // serves at full speed. The direct URL is the fallback when the proxy is off.
    let url = crate::audioproxy::register(&data.stream_url, &data.headers)
        .unwrap_or_else(|| data.stream_url.clone());
    let client = reqwest::Client::builder().no_proxy().build().map_err(|e| e.to_string())?;
    let mut req = client.get(&url);
    if !url.starts_with("http://127.0.0.1") {
        for (k, v) in &data.headers {
            req = req.header(k, v);
        }
    }
    let mut resp = req.send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("the stream answered {}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(0);
    let ext = match resp.headers().get("content-type").and_then(|v| v.to_str().ok()) {
        Some(ct) if ct.contains("mp4") => "m4a",
        Some(ct) if ct.contains("mpeg") => "mp3",
        _ => "webm",
    };
    let dir = dir(&state.app);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;
    let part = dir.join(format!("{}.part", song.video_id));
    let path = dir.join(format!("{}.{ext}", song.video_id));
    let mut file = tokio::fs::File::create(&part).await.map_err(|e| e.to_string())?;
    let mut done: u64 = 0;
    let mut last_emit = 0u64;
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        done += chunk.len() as u64;
        if done - last_emit > 256 * 1024 {
            last_emit = done;
            queue().lock().unwrap().progress = Some((done, total));
            let _ = state.app.emit(
                "download-progress",
                serde_json::json!({ "videoId": song.video_id, "done": done, "total": total }),
            );
        }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);
    if total > 0 && done < total {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(format!("the download stopped at {done} of {total} bytes"));
    }
    tokio::fs::rename(&part, &path).await.map_err(|e| e.to_string())?;
    Ok(Entry {
        song: song.clone(),
        file: path.to_string_lossy().into_owned(),
        bytes: done,
        added_at: now_secs(),
        loudness_db: data.loudness_db,
    })
}

pub fn remove(state: &AppState, ids: &[String]) {
    let mut map = load(&state.db);
    for id in ids {
        if let Some(e) = map.remove(id) {
            let _ = std::fs::remove_file(&e.file);
        }
    }
    save(&state.db, &map);
    {
        let mut q = queue().lock().unwrap();
        // The one in flight stays: it finishes, and can be removed again once it is in the list.
        let running = q.running;
        let first = q.waiting.front().map(|s| s.video_id.clone());
        q.waiting.retain(|s| !ids.contains(&s.video_id) || (running && Some(&s.video_id) == first.as_ref()));
    }
    changed(&state.app, &state.db);
}

// ---- commands ---------------------------------------------------------------------------------

type St<'a> = tauri::State<'a, Arc<AppState>>;

#[tauri::command]
pub fn get_downloads(state: St<'_>) -> Status {
    status(&state.db)
}

#[tauri::command]
pub fn download_songs(state: St<'_>, songs: Vec<SongItem>) {
    enqueue(state.inner().clone(), songs);
}

#[tauri::command]
pub fn remove_downloads(state: St<'_>, video_ids: Vec<String>) {
    remove(&state, &video_ids);
}
