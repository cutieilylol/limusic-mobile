package com.limusic.mobile

import androidx.annotation.OptIn
import androidx.media3.common.ForwardingPlayer
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import org.json.JSONObject

/**
 * What the media session (notification, lock screen, headset, Bluetooth, Wear, Auto) sees: the
 * live ExoPlayer deck for state, position and metadata, with every transport button routed to the
 * Rust queue instead of the deck. The queue is the app's, not ExoPlayer's (the deck only ever
 * holds "current + next"), so "next" has to mean the app's next: shuffle, radio and autoplay
 * included. Same reason the desktop media keys go through `AppState`.
 */
@OptIn(UnstableApi::class)
class RemotePlayer(player: Player) : ForwardingPlayer(player) {
    private fun remote(action: String, pos: Double? = null) {
        val o = JSONObject().put("action", action)
        if (pos != null) o.put("pos", pos)
        LimusicBridge.emit("remote", o)
    }

    override fun getAvailableCommands(): Player.Commands =
        super.getAvailableCommands().buildUpon()
            .addAll(
                Player.COMMAND_PLAY_PAUSE,
                Player.COMMAND_SEEK_TO_NEXT,
                Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM,
                Player.COMMAND_SEEK_TO_PREVIOUS,
                Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM,
                Player.COMMAND_SEEK_IN_CURRENT_MEDIA_ITEM,
                Player.COMMAND_STOP,
            )
            .build()

    override fun isCommandAvailable(command: Int): Boolean = availableCommands.contains(command)

    override fun play() = remote("play")
    override fun pause() = remote("pause")
    override fun setPlayWhenReady(playWhenReady: Boolean) = remote(if (playWhenReady) "play" else "pause")
    override fun stop() = remote("pause")

    override fun seekToNext() = remote("next")
    override fun seekToNextMediaItem() = remote("next")
    override fun seekToPrevious() = remote("prev")
    override fun seekToPreviousMediaItem() = remote("prev")
    override fun hasNextMediaItem() = true
    override fun hasPreviousMediaItem() = true

    override fun seekTo(positionMs: Long) = remote("seek", positionMs / 1000.0)
    override fun seekTo(mediaItemIndex: Int, positionMs: Long) = remote("seek", positionMs / 1000.0)
    override fun seekBack() = remote("seek", maxOf(0L, currentPosition - seekBackIncrement) / 1000.0)
    override fun seekForward() = remote("seek", (currentPosition + seekForwardIncrement) / 1000.0)
}
