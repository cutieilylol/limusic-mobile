//! The audio player, one API on every platform.
//!
//! Desktop: libmpv (`mpv.rs`, context/14). Phones: decks living on the native side, Media3
//! ExoPlayer on Android (`gen/android/.../PlayerEngine.kt`) and AVFoundation on iOS
//! (`gen/apple/.../PlayerEngine.swift`), driven by `native.rs` over the bridge. All honour the same
//! contract: a 1-track lookahead for gapless transitions, a second deck while crossfading, and the
//! same [`PlayerEvent`] stream back to the app.

#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod mpv;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub use mpv::*;

#[cfg(any(target_os = "android", target_os = "ios"))]
mod native;
#[cfg(any(target_os = "android", target_os = "ios"))]
pub use native::*;
