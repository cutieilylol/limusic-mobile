package com.limusic.mobile

import android.content.Context
import android.media.AudioAttributes as PlatformAudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.annotation.OptIn
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.PlaybackException
import androidx.media3.common.PlaybackParameters
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.common.util.Util
import androidx.media3.datasource.DefaultDataSource
import androidx.media3.datasource.DefaultHttpDataSource
import androidx.media3.exoplayer.DefaultLoadControl
import androidx.media3.exoplayer.DefaultRenderersFactory
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.audio.AudioSink
import androidx.media3.exoplayer.audio.DefaultAudioSink
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import org.json.JSONObject
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.min
import kotlin.math.max
import kotlin.math.roundToLong
import kotlin.math.sin

/**
 * The Android audio engine: ExoPlayer decks behind the same contract `crates/player/src/mpv.rs`
 * keeps with libmpv, so the Rust state machine above it runs unchanged.
 *
 * - **Gapless**: the lookahead is appended to the playing deck's own playlist. ExoPlayer opens it
 *   ahead of the boundary and the audio track runs straight through, trimming Opus pre-skip and
 *   discard padding from the WebM container.
 * - **Crossfade**: a second deck holds the next track paused and silent; [startCrossfade] swaps
 *   which deck is live and ramps both with an equal-power (cos/sin) curve. `TrackEnded` goes out
 *   when the overlap *starts*, as on desktop.
 * - **Pause survives a load**, like mpv's `pause` property: `load` never touches playWhenReady.
 * - Only the live deck's events reach Rust.
 *
 * Audio focus is held here, once for the app, not per deck: two ExoPlayers each handling focus
 * would take it from each other the moment a crossfade starts.
 */
@OptIn(UnstableApi::class)
object PlayerEngine {
    private const val TAG = "PlayerEngine"
    private const val FADE_STEP_MS = 50L
    private const val TICK_MS = 250L

    private val main = Handler(Looper.getMainLooper())
    private val decks = arrayOfNulls<Deck>(2)
    private var active = 0

    private var crossfadeMs = 0L
    /** The user's volume as a linear amplitude (the Rust side does the perceptual curve). */
    private var userAmp = 1f
    private var gainDb = 0.0
    private var speed = 1f
    private var pitch = 1f
    private var loopFile = false
    private var skipSilence = false
    private var headers: Map<String, String> = emptyMap()

    // Crossfade bookkeeping, mirroring `Decks` in mpv.rs.
    private var preloaded = false
    private var preloading = false
    private var preloadGen = 0L
    private var preloadArmed = -1L
    private var fading = false
    private var fadeGen = 0L
    private var pending: String? = null

    private var meta: MediaMetadata? = null
    private var audioSession = C.AUDIO_SESSION_ID_UNSET

    /** Set by [PlaybackService] so a deck swap moves the media session onto the new live deck. */
    var onSessionPlayer: ((Player) -> Unit)? = null
    private var sessionPlayer: RemotePlayer? = null

    private val ctx: Context get() = LimusicApp.instance

    // ---- commands from Rust -------------------------------------------------------------------

    fun command(cmd: String, o: JSONObject) {
        when (cmd) {
            "player.init" -> deck(0)
            "player.load" -> load(
                o.optString("url"),
                o.optJSONObject("headers"),
                if (o.isNull("gain")) null else o.optDouble("gain"),
                if (o.isNull("start")) null else o.optDouble("start"),
            )
            "player.enqueue" -> enqueue(o.optString("url"))
            "player.crossfade" -> setCrossfade(o.optDouble("secs", 0.0))
            "player.clearPlaylist" -> clearPlaylist()
            "player.stop" -> stop()
            "player.play" -> play()
            "player.pause" -> pause()
            "player.toggle" -> if (live().exo.playWhenReady) pause() else play()
            "player.loop" -> setLoop(o.optBoolean("on"))
            "player.seek" -> seek(o.optDouble("pos"))
            "player.volume" -> setVolume(o.optDouble("gain", 1.0).toFloat())
            "player.gain" -> setGain(o.optDouble("db", 0.0))
            "player.speed" -> { speed = o.optDouble("speed", 1.0).toFloat(); applyParams() }
            "player.pitch" -> { pitch = o.optDouble("factor", 1.0).toFloat(); applyParams() }
            "player.skipSilence" -> {
                skipSilence = o.optBoolean("on")
                decks.forEach { it?.exo?.skipSilenceEnabled = skipSilence }
            }
            "media.metadata" -> setMetadata(o)
            "media.album" -> {
                meta = (meta?.buildUpon() ?: MediaMetadata.Builder())
                    .setAlbumTitle(o.optString("album")).build()
                applyMetadata()
            }
        }
    }

    fun audioSessionId(): Int {
        deck(0)
        return audioSession
    }

    fun sessionPlayer(): Player {
        deck(0)
        return sessionPlayer ?: RemotePlayer(live().exo).also { sessionPlayer = it }
    }

    private fun load(url: String, hdrs: JSONObject?, gain: Double?, start: Double?) {
        dropPreload(true)
        headers = hdrs?.let { h -> h.keys().asSequence().associateWith { h.optString(it) } } ?: emptyMap()
        val d = live()
        d.lastDuration = -1.0
        d.http.setDefaultRequestProperties(headers.filterKeys { !it.equals("user-agent", true) })
        headers.entries.firstOrNull { it.key.equals("user-agent", true) }?.let { d.http.setUserAgent(it.value) }
        setGain(gain ?: 0.0)
        val startMs = start?.let { (it * 1000).roundToLong() } ?: 0L
        d.exo.setMediaItem(item(url), startMs)
        d.exo.prepare()
        if (d.exo.playWhenReady) requestFocus()
    }

    private fun enqueue(url: String) {
        if (crossfadeMs == 0L) {
            live().exo.addMediaItem(item(url))
            return
        }
        if (fading) {
            pending = url
            return
        }
        preload(1 - active, url)
    }

    private fun setCrossfade(secs: Double) {
        crossfadeMs = (secs * 1000).toLong()
        if (crossfadeMs == 0L) dropPreload(false)
    }

    private fun clearPlaylist() {
        val exo = live().exo
        val cur = exo.currentMediaItemIndex
        if (exo.mediaItemCount > cur + 1) exo.removeMediaItems(cur + 1, exo.mediaItemCount)
        if (cur > 0) exo.removeMediaItems(0, cur)
        dropPreload(false)
    }

    private fun stop() {
        dropPreload(true)
        val exo = live().exo
        exo.stop()
        exo.clearMediaItems()
        abandonFocus()
    }

    private fun play() {
        val exo = live().exo
        requestFocus()
        if (exo.playbackState == Player.STATE_IDLE && exo.mediaItemCount > 0) exo.prepare()
        exo.playWhenReady = true
    }

    private fun pause() {
        cancelFade()
        live().exo.playWhenReady = false
        resumeOnFocusGain = false
    }

    private fun setLoop(on: Boolean) {
        loopFile = on
        live().exo.repeatMode = if (on) Player.REPEAT_MODE_ONE else Player.REPEAT_MODE_OFF
    }

    private fun seek(pos: Double) {
        cancelFade()
        live().exo.seekTo((pos * 1000).roundToLong())
    }

    private fun setVolume(amp: Float) {
        userAmp = amp.coerceIn(0f, 1f)
        if (!fading) live().exo.volume = userAmp * duckFactor
    }

    private fun setGain(db: Double) {
        gainDb = db
        live().gain.setDb(db)
    }

    private fun applyParams() {
        val p = PlaybackParameters(speed.coerceIn(0.25f, 2f), pitch.coerceIn(0.5f, 2f))
        decks.forEach { it?.exo?.playbackParameters = p }
    }

    // ---- media session metadata ---------------------------------------------------------------

    private fun setMetadata(o: JSONObject) {
        val b = MediaMetadata.Builder()
            .setTitle(o.optString("title"))
            .setArtist(o.optString("artist"))
            .setDisplayTitle(o.optString("title"))
            .setSubtitle(o.optString("artist"))
            .setMediaType(MediaMetadata.MEDIA_TYPE_MUSIC)
            .setIsPlayable(true)
        if (!o.isNull("album")) b.setAlbumTitle(o.optString("album"))
        if (!o.isNull("cover")) {
            val cover = o.optString("cover")
            if (cover.startsWith("http")) b.setArtworkUri(Uri.parse(cover))
        }
        meta = b.build()
        applyMetadata()
    }

    /** Put the metadata on the live deck's current item. Same URI, so ExoPlayer swaps it in place
     *  without touching playback. */
    private fun applyMetadata() {
        val m = meta ?: return
        val exo = live().exo
        val cur = exo.currentMediaItem ?: return
        if (cur.mediaMetadata == m) return
        exo.replaceMediaItem(exo.currentMediaItemIndex, cur.buildUpon().setMediaMetadata(m).build())
    }

    // ---- decks --------------------------------------------------------------------------------

    private fun live(): Deck = deck(active)

    private fun deck(i: Int): Deck {
        decks[i]?.let { return it }
        if (audioSession == C.AUDIO_SESSION_ID_UNSET) {
            audioSession = Util.generateAudioSessionIdV21(ctx)
        }
        val d = Deck(i)
        decks[i] = d
        return d
    }

    private fun item(url: String): MediaItem {
        val b = MediaItem.Builder().setUri(url).setMediaId(url)
        meta?.let { b.setMediaMetadata(it) }
        return b.build()
    }

    private fun isLive(d: Deck) = decks[active] === d

    private fun preload(i: Int, url: String) {
        val d = deck(i)
        d.exo.playWhenReady = false
        d.exo.volume = 0f
        d.gain.setDb(gainDb)
        d.lastDuration = -1.0
        preloadArmed = preloadGen
        preloaded = false
        preloading = true
        d.exo.setMediaItem(item(url))
        d.exo.prepare()
    }

    private fun dropPreload(cutFade: Boolean) {
        pending = null
        preloadGen++
        if (cutFade) cancelFade()
        if ((preloaded || preloading) && !fading) {
            decks[1 - active]?.exo?.let { it.stop(); it.clearMediaItems() }
        }
        preloaded = false
        preloading = false
    }

    private fun cancelFade() {
        if (!fading) return
        fadeGen++
        decks[1 - active]?.exo?.playWhenReady = false
    }

    /** See `fade_due` in mpv.rs: never longer than half the track, nor than what is left. */
    private fun fadeDue(settingS: Double, pos: Double, dur: Double): Double? {
        if (!(settingS > 0 && pos.isFinite() && dur.isFinite() && dur > 1.0)) return null
        val left = dur - pos
        val fade = min(settingS, dur / 2)
        return if (left <= fade) max(0.0, min(fade, left)) else null
    }

    private fun maybeCrossfade(d: Deck, pos: Double) {
        if (loopFile || fading || !preloaded || crossfadeMs == 0L) return
        val fade = fadeDue(crossfadeMs / 1000.0, pos, d.durationSecs()) ?: return
        startCrossfade(d.index, fade)
    }

    private fun startCrossfade(from: Int, fadeIn: Double) {
        val out = decks[from] ?: return
        val inc = decks[1 - from] ?: return
        preloaded = false
        preloading = false
        val gen = fadeGen
        fading = true
        active = 1 - from
        swapSession()
        inc.exo.playWhenReady = true
        inc.lastDuration = -1.0
        emit("ended", inc)
        var fade = fadeIn
        val incDur = inc.durationSecs()
        if (incDur > 0) {
            inc.lastDuration = incDur
            emit("duration", inc, incDur)
            fade = min(fade, incDur / 2)
        }
        applyMetadata()
        Log.i(TAG, "crossfading from deck $from over ${fade}s")
        val steps = max(1L, (fade * 1000 / FADE_STEP_MS).roundToLong())
        var step = 0L
        val ramp = object : Runnable {
            override fun run() {
                if (fadeGen != gen) {
                    finishFade(out, inc, true)
                    return
                }
                step++
                val turn = step.toDouble() / steps * (PI / 2)
                out.exo.volume = (userAmp * duckFactor * cos(turn)).toFloat().coerceAtLeast(0f)
                inc.exo.volume = (userAmp * duckFactor * sin(turn)).toFloat().coerceAtLeast(0f)
                if (step >= steps) finishFade(out, inc, false) else main.postDelayed(this, FADE_STEP_MS)
            }
        }
        main.postDelayed(ramp, FADE_STEP_MS)
    }

    private fun finishFade(out: Deck, inc: Deck, cancelled: Boolean) {
        out.exo.stop()
        out.exo.clearMediaItems()
        out.exo.volume = userAmp * duckFactor
        if (cancelled) inc.exo.volume = userAmp * duckFactor
        fading = false
        val next = pending
        pending = null
        if (next != null && crossfadeMs > 0) preload(1 - active, next)
    }

    private fun swapSession() {
        val p = RemotePlayer(live().exo)
        sessionPlayer = p
        onSessionPlayer?.invoke(p)
    }

    // ---- events to Rust -----------------------------------------------------------------------

    private fun emit(ev: String, d: Deck, v: Any? = null, msg: String? = null) {
        val o = JSONObject().put("ev", ev).put("idle", d.idle())
        o.put("path", d.exo.currentMediaItem?.mediaId ?: JSONObject.NULL)
        if (v != null) o.put("v", v)
        if (msg != null) o.put("msg", msg)
        LimusicBridge.emit("player", o)
    }

    private val ticker = object : Runnable {
        override fun run() {
            val d = decks[active]
            if (d != null && d.exo.isPlaying) {
                val pos = d.exo.currentPosition / 1000.0
                emit("position", d, pos)
                maybeCrossfade(d, pos)
                main.postDelayed(this, TICK_MS)
            } else {
                ticking = false
            }
        }
    }
    private var ticking = false

    private fun startTicker() {
        if (ticking) return
        ticking = true
        main.postDelayed(ticker, TICK_MS)
    }

    // ---- audio focus --------------------------------------------------------------------------

    private val audioManager by lazy { ctx.getSystemService(Context.AUDIO_SERVICE) as AudioManager }
    private var focusRequest: AudioFocusRequest? = null
    private var hasFocus = false
    private var resumeOnFocusGain = false
    private var duckFactor = 1f

    private val focusListener = AudioManager.OnAudioFocusChangeListener { change ->
        main.post {
            when (change) {
                AudioManager.AUDIOFOCUS_GAIN -> {
                    hasFocus = true
                    setDuck(1f)
                    if (resumeOnFocusGain) {
                        resumeOnFocusGain = false
                        live().exo.playWhenReady = true
                    }
                }
                AudioManager.AUDIOFOCUS_LOSS -> {
                    hasFocus = false
                    resumeOnFocusGain = false
                    cancelFade()
                    live().exo.playWhenReady = false
                }
                AudioManager.AUDIOFOCUS_LOSS_TRANSIENT -> {
                    // A call, a voice assistant: pause, and pick up again when it is over.
                    resumeOnFocusGain = live().exo.playWhenReady
                    cancelFade()
                    live().exo.playWhenReady = false
                }
                AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK -> setDuck(0.2f)
            }
        }
    }

    private fun setDuck(f: Float) {
        duckFactor = f
        if (!fading) live().exo.volume = userAmp * duckFactor
    }

    private fun requestFocus() {
        if (hasFocus) return
        val res = if (Build.VERSION.SDK_INT >= 26) {
            val req = focusRequest ?: AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                .setAudioAttributes(
                    PlatformAudioAttributes.Builder()
                        .setUsage(PlatformAudioAttributes.USAGE_MEDIA)
                        .setContentType(PlatformAudioAttributes.CONTENT_TYPE_MUSIC)
                        .build()
                )
                .setOnAudioFocusChangeListener(focusListener, main)
                .setWillPauseWhenDucked(false)
                .build().also { focusRequest = it }
            audioManager.requestAudioFocus(req)
        } else {
            @Suppress("DEPRECATION")
            audioManager.requestAudioFocus(
                focusListener, AudioManager.STREAM_MUSIC, AudioManager.AUDIOFOCUS_GAIN
            )
        }
        hasFocus = res == AudioManager.AUDIOFOCUS_REQUEST_GRANTED
    }

    private fun abandonFocus() {
        if (!hasFocus) return
        hasFocus = false
        if (Build.VERSION.SDK_INT >= 26) {
            focusRequest?.let { audioManager.abandonAudioFocusRequest(it) }
        } else {
            @Suppress("DEPRECATION")
            audioManager.abandonAudioFocus(focusListener)
        }
    }

    // ---- one deck -----------------------------------------------------------------------------

    private class GainRenderers(ctx: Context, private val gain: GainProcessor) :
        DefaultRenderersFactory(ctx) {
        override fun buildAudioSink(
            context: Context,
            enableFloatOutput: Boolean,
            enableAudioTrackPlaybackParams: Boolean,
        ): AudioSink = DefaultAudioSink.Builder(context)
            .setAudioProcessors(arrayOf(gain))
            .setEnableFloatOutput(enableFloatOutput)
            .setEnableAudioTrackPlaybackParams(enableAudioTrackPlaybackParams)
            .build()
    }

    private class Deck(val index: Int) {
        val gain = GainProcessor()
        val exo: ExoPlayer
        val http: DefaultHttpDataSource.Factory
        var lastDuration = -1.0
        var paused = true
        var playing = false

        init {
            val ctx = LimusicApp.instance
            http = DefaultHttpDataSource.Factory()
                .setAllowCrossProtocolRedirects(true)
                .setConnectTimeoutMs(15_000)
                .setReadTimeoutMs(30_000)
                .setUserAgent("Limusic")
            val dataSource = DefaultDataSource.Factory(ctx, http)
            exo = ExoPlayer.Builder(ctx, GainRenderers(ctx, gain))
                .setMediaSourceFactory(DefaultMediaSourceFactory(dataSource))
                .setLoadControl(
                    DefaultLoadControl.Builder()
                        // Keep what was heard, so scrubbing back is instant (mpv.rs keeps 64 MiB).
                        .setBackBuffer(120_000, true)
                        .setBufferDurationsMs(30_000, 120_000, 1_000, 2_000)
                        .build()
                )
                .setAudioAttributes(
                    AudioAttributes.Builder()
                        .setUsage(C.USAGE_MEDIA)
                        .setContentType(C.AUDIO_CONTENT_TYPE_MUSIC)
                        .build(),
                    false, // focus is the engine's, see the class comment
                )
                .setHandleAudioBecomingNoisy(true) // headphones out: pause
                .setWakeMode(C.WAKE_MODE_NETWORK)
                .build()
            exo.setAudioSessionId(audioSession)
            exo.skipSilenceEnabled = skipSilence
            exo.playbackParameters = PlaybackParameters(speed, pitch)
            exo.addListener(Listener(this))
        }

        fun idle(): Boolean {
            val s = exo.playbackState
            return exo.mediaItemCount == 0 || s == Player.STATE_IDLE || s == Player.STATE_ENDED
        }

        fun durationSecs(): Double {
            val d = exo.duration
            return if (d == C.TIME_UNSET || d <= 0) -1.0 else d / 1000.0
        }

        fun reportPlaying() {
            val now = !paused && !idle()
            if (now != playing) {
                playing = now
                if (isLive(this)) emit("playing", this, now)
            }
            if (now && isLive(this)) startTicker()
        }

        fun reportDuration() {
            val d = durationSecs()
            if (d > 0 && d != lastDuration && isLive(this)) {
                lastDuration = d
                emit("duration", this, d)
            }
        }
    }

    private class Listener(val d: Deck) : Player.Listener {
        override fun onPlayWhenReadyChanged(playWhenReady: Boolean, reason: Int) {
            val p = !playWhenReady
            if (p != d.paused) {
                d.paused = p
                if (isLive(d)) emit("paused", d, p)
            }
            if (playWhenReady && isLive(d)) requestFocus()
            d.reportPlaying()
        }

        override fun onPlaybackStateChanged(state: Int) {
            when (state) {
                Player.STATE_READY -> {
                    if (!isLive(d) && preloading && preloadArmed == preloadGen) {
                        preloading = false
                        preloaded = true
                    }
                    d.reportDuration()
                }
                Player.STATE_ENDED -> if (isLive(d)) emit("ended", d)
            }
            d.reportPlaying()
        }

        override fun onTimelineChanged(timeline: androidx.media3.common.Timeline, reason: Int) {
            d.reportDuration()
        }

        override fun onMediaItemTransition(mediaItem: MediaItem?, reason: Int) {
            if (reason != Player.MEDIA_ITEM_TRANSITION_REASON_AUTO || !isLive(d)) return
            // The gapless advance: the lookahead is now playing. Drop what is behind it, so the
            // deck's playlist is only ever "current + next" like mpv's.
            val cur = d.exo.currentMediaItemIndex
            if (cur > 0) d.exo.removeMediaItems(0, cur)
            d.lastDuration = -1.0
            emit("ended", d)
            d.reportDuration()
        }

        override fun onPositionDiscontinuity(
            oldPosition: Player.PositionInfo,
            newPosition: Player.PositionInfo,
            reason: Int,
        ) {
            if (reason == Player.DISCONTINUITY_REASON_SEEK && isLive(d)) {
                emit("position", d, d.exo.currentPosition / 1000.0)
            }
        }

        override fun onPlayerError(error: PlaybackException) {
            val msg = friendly(error)
            when {
                isLive(d) -> {
                    Log.w(TAG, "track failed: ${error.errorCodeName}", error)
                    emit("failed", d, msg = msg)
                }
                fading -> Log.w(TAG, "outgoing track died mid-fade: ${error.errorCodeName}")
                else -> {
                    Log.w(TAG, "crossfade preload failed: ${error.errorCodeName}")
                    preloaded = false
                    preloading = false
                    emit("lookaheadFailed", d, msg = msg)
                }
            }
            d.reportPlaying()
        }
    }

    private fun friendly(e: PlaybackException): String = when (e.errorCode) {
        PlaybackException.ERROR_CODE_IO_BAD_HTTP_STATUS,
        PlaybackException.ERROR_CODE_IO_NETWORK_CONNECTION_FAILED,
        PlaybackException.ERROR_CODE_IO_NETWORK_CONNECTION_TIMEOUT ->
            "Couldn't load this track. The stream link was refused."
        PlaybackException.ERROR_CODE_PARSING_CONTAINER_UNSUPPORTED,
        PlaybackException.ERROR_CODE_DECODING_FORMAT_UNSUPPORTED -> "Unrecognized audio format"
        PlaybackException.ERROR_CODE_AUDIO_TRACK_INIT_FAILED -> "Couldn't start audio output"
        else -> "Playback failed (${e.errorCodeName})"
    }
}
