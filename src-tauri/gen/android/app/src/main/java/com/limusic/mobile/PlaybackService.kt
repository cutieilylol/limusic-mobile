package com.limusic.mobile

import android.app.PendingIntent
import android.content.Intent
import androidx.annotation.OptIn
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService

/**
 * Keeps the music going with the app in the background or the screen off, and owns the media
 * session behind the notification and the lock-screen player. Media3 promotes the service to the
 * foreground (with its media notification) while something plays, and lets it go when paused.
 *
 * The players themselves belong to [PlayerEngine], which outlives this service: the session is
 * the only thing created and released here.
 */
@OptIn(UnstableApi::class)
class PlaybackService : MediaSessionService() {
    private var session: MediaSession? = null

    override fun onCreate() {
        super.onCreate()
        val open = PendingIntent.getActivity(
            this, 0,
            Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val s = MediaSession.Builder(this, PlayerEngine.sessionPlayer())
            .setSessionActivity(open)
            .build()
        session = s
        PlayerEngine.onSessionPlayer = { p -> s.player = p }
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? = session

    /** Swiped away from recents: keep playing if something is (like YouTube Music and Spotify),
     *  otherwise there is nothing left to do in the background. */
    override fun onTaskRemoved(rootIntent: Intent?) {
        val p = session?.player
        if (p == null || !p.playWhenReady || p.mediaItemCount == 0) {
            stopSelf()
        }
    }

    override fun onDestroy() {
        PlayerEngine.onSessionPlayer = null
        session?.release()
        session = null
        super.onDestroy()
    }
}
