# Limusic on iPhone

The iOS build is the same app as the Android one: the Rust core (`src-tauri`, `crates/*`) and the
SvelteKit UI (`ui`), packaged by Tauri 2. See `docs/ANDROID.md` for the shape of the phone build.
What differs is the native side, which here is Swift (`src-tauri/gen/apple/Sources/limusic-app`).

| Android | iOS |
|---|---|
| Media3 ExoPlayer decks (`PlayerEngine.kt`) | AVQueuePlayer decks (`PlayerEngine.swift`) |
| Media session, notification | `MPNowPlayingInfoCenter`, `MPRemoteCommandCenter` |
| Off-screen Android WebViews | Off-screen WKWebViews (`HiddenWebViews.swift`) |
| `LoginActivity` | `LoginViewController` (`LoginFlow.swift`) |
| Discord through the browser and an intent filter | `ASWebAuthenticationSession` |
| The phone's Music folder | The app's own `Documents/Music`, visible in the Files app |
| JNI (`bridge.rs`) | A C ABI pair (`bridge.rs`, `LimusicBridge.swift`) |

`crates/player/src/bridge.rs` carries the same JSON commands and events on both, so everything
above it, `native.rs`, `src-tauri/src/mobile/*` and `phone.rs`, is shared.

Audio differs from Android in three ways. YouTube's Opus streams are WebM, which AVFoundation can't
open, so iOS plays the AAC ones (`plays_here` in `crates/innertube`). An AVPlayer's volume stops at
1, so loudness normalization only turns tracks down. Pitch and skip silence need an audio engine
of their own and aren't there yet; the pitch control reports that and the setting is hidden.

## Building it

Xcode only runs on a Mac, so the build is the `iOS` GitHub Actions workflow, started by hand:
`mode=ipa` for the installable app, `mode=sim` for a debug run in the Simulator that comes back with
a screenshot and the app's output.

`src-tauri/gen/apple` is committed apart from the Xcode project, which `xcodegen` makes from
`project.yml` on each run. Don't run `tauri ios init` over it: it would replace `project.yml` and
`main.mm`. The workflow builds the Rust library itself, because Tauri's Xcode build step needs its
CLI running around `xcodebuild`, which only `tauri ios build` with a signing team sets up.

## Installing it

The `.ipa` is unsigned, and an iPhone won't run it until it has been signed for that phone.
[Sideloadly](https://sideloadly.io) does that from Windows with an Apple ID. It needs iTunes from
Apple's site rather than the Microsoft Store. With a free Apple ID the app stops opening after 7
days until it is installed again, and at most 3 sideloaded apps can be active.
