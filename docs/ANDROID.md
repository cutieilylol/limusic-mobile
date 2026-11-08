# Limusic on Android

The Android build is the same app: the same Rust core (`src-tauri`, `crates/*`) and the same
SvelteKit UI (`ui`), packaged by Tauri 2 for Android. What changes is the shell around them.

## What is different on a phone

| Desktop | Android |
|---|---|
| libmpv plays the audio (`crates/player/src/mpv.rs`) | Media3 ExoPlayer plays it (`gen/android/.../PlayerEngine.kt`), behind the same `player::Player` API (`crates/player/src/android.rs`): gapless lookahead in the deck's playlist, two decks for crossfade, loudness gain with a peak limiter, tempo and pitch |
| MPRIS / SMTC media controls (`media.rs`) | A Media3 media session (`PlaybackService.kt`): notification, lock screen, Bluetooth and headset buttons, all routed to the app's own queue (`RemotePlayer.kt`) |
| Hidden Tauri windows for the cipher and the session refresh | Off-screen Android WebViews (`HiddenWebViews.kt`, `LoginFlow.kt`), behind the same Rust APIs (`src-tauri/src/mobile/webview.rs`, `mobile/session.rs`) |
| Sign-in window | A full-screen sign-in screen (`LoginActivity` in `LoginFlow.kt`) |
| BotGuard PoTokens (V8) | None: rusty_v8 has no Android build. Streams resolve through the clients that need no token (VISIONOS first), the same fallback desktop takes when its JS runtime is down |
| Titlebar, sidebar, player bar, floating panels | Top bar, bottom navigation, mini player, full-screen player, bottom sheets (`Mobile*.svelte`) |
| Discord presence through the local Discord client's IPC socket | The same presence code (`discord.rs`), sent over Discord's Gateway as the signed-in user, after an OAuth2 + PKCE sign-in through the user's own Discord app (`mobile/discord_net.rs`, `DiscordOAuthActivity.kt`): Metrolist's approach |
| Tray, mini player window, global hotkeys, autostart, updater | Not on Android (stubs in `src-tauri/src/mobile/`) |

Phone-only extras: sleep timer (`phone.rs`, runs in Rust so it fires with the screen off), offline
downloads (`downloads.rs`, Library ▸ Downloads), skip silence, the system equalizer, the Android
share sheet, swipe gestures on the mini player and the player, long-press menus, drag-to-reorder
by the grip in Up next, Play and Shuffle side by side on playlists, keep-screen-on while lyrics are
open, and the phone's Music folder in Local Music.

The UI is the desktop's, laid out for a phone by the `phone:` Tailwind variant (`:root.android`,
set by `MainActivity`) and the Android block at the end of `layout.css`: library and local
collections as rows instead of card grids, stacked page headers, menus and dialogs as bottom
sheets, and the keyboard shrinking the page (a bottom margin on the WebView) with the mini player
and the navigation bar hidden while it is up.

The Rust ⇄ Kotlin seam is one JNI bridge (`crates/player/src/bridge.rs` ⇄ `LimusicBridge.kt`):
JSON commands one way, JSON events the other.

## Building it on Windows

Needs: Rust (with the `aarch64-linux-android` target), Node + pnpm, the Tauri CLI
(`npm i -g @tauri-apps/cli@2`), a JDK 17+, and the Android SDK with `platforms;android-36`,
`build-tools;36.1.0` and NDK 28 (`ndk;28.2.13676358`).

```bash
export ANDROID_HOME='C:\Android\sdk'
export NDK_HOME='C:\Android\sdk\ndk\28.2.13676358'
scripts/android-build.sh          # quick build (no LTO)
FULL=1 scripts/android-build.sh   # the real release profile
```

The script exists because `tauri android build` symlinks the native library into `jniLibs`, which
Windows refuses without Developer Mode: it lets Tauri compile, copies the library in itself and
runs Gradle with Gradle's own Rust step skipped. The APK lands in
`src-tauri/gen/android/app/build/outputs/apk/arm64/release/`, signed with the local key named in
`src-tauri/gen/android/keystore.properties` (not in git).

`rquickjs-sys` (rustypipe's JS engine) ships no Android bindings; `vendor/rquickjs-sys` is a copy
carrying the linux-gnu ones under the Android target names (same LP64 ABI), wired up with
`[patch.crates-io]` in the workspace `Cargo.toml`.

## Discord status on the phone

Desktop hands the card to the Discord app running on the same computer. A phone has no such app to
talk to, so Limusic signs in to Discord itself, with Discord's own OAuth page, and keeps a Gateway
connection open while presence is on. Discord only allows that through a Discord application the
user owns, so it needs a one-time setup:

1. Open <https://discord.com/developers/applications>, sign in, and press **New Application**.
   Name it what you want your friends to see after "Listening to" (for example *Limusic*).
2. Optional: on **General Information**, upload an app icon.
3. Copy the **Application ID** from **General Information**.
4. Open **OAuth2**. Under **Redirects**, press **Add Redirect**, paste
   `limusicdiscord://oauth2/callback`, and save. On the same page, switch on **Public Client** and
   save again.
5. If the sidebar shows a **Social SDK** section, open it and enable the Social SDK for the app.
   Sign-in uses its `sdk.social_layer_presence` scope; an "invalid scope" error means this step is
   missing.
6. In Limusic: **Settings ▸ Discord**, paste the Application ID, **Save**, then **Sign in with
   Discord**, approve on Discord's page, and switch **Rich Presence** on.

The access token lasts about a week and is refreshed by itself. **Sign out** forgets it.
