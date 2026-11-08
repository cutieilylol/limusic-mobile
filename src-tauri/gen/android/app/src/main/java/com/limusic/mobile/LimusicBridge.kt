package com.limusic.mobile

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.media.audiofx.AudioEffect
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import org.json.JSONObject
import java.lang.ref.WeakReference

/**
 * The JNI seam between the Rust core and the Android side (Rust half: crates/player/src/bridge.rs).
 *
 * Rust calls [command] from any thread with one JSON object; the work is posted to the main
 * looper, in order, and the call returns at once. Everything here that touches ExoPlayer, a
 * WebView or an Activity therefore runs on the main thread. Events go back through
 * [nativeEvent], tagged with a `topic` the Rust side routes on.
 */
object LimusicBridge {
    private const val TAG = "LimusicBridge"
    private val main = Handler(Looper.getMainLooper())

    /** The activity on screen, for the few commands that need one (immersive mode, intents). */
    @Volatile
    var activity: WeakReference<Activity>? = null

    @JvmStatic
    external fun nativeInit()

    @JvmStatic
    external fun nativeEvent(json: String)

    @JvmStatic
    fun command(json: String) {
        val obj = try {
            JSONObject(json)
        } catch (e: Exception) {
            Log.w(TAG, "bad command json", e)
            return
        }
        main.post {
            try {
                dispatch(obj)
            } catch (e: Throwable) {
                Log.e(TAG, "command ${obj.optString("cmd")} failed", e)
            }
        }
    }

    fun emit(topic: String, obj: JSONObject) {
        obj.put("topic", topic)
        try {
            nativeEvent(obj.toString())
        } catch (e: UnsatisfiedLinkError) {
            Log.w(TAG, "event before the native library was loaded: $topic")
        }
    }

    fun musicDir(): String =
        android.os.Environment.getExternalStoragePublicDirectory(android.os.Environment.DIRECTORY_MUSIC).absolutePath

    private fun dispatch(o: JSONObject) {
        val cmd = o.optString("cmd")
        when {
            cmd.startsWith("player.") || cmd.startsWith("media.") -> PlayerEngine.command(cmd, o)
            cmd.startsWith("webview.") -> HiddenWebViews.command(cmd, o)
            cmd.startsWith("login.") -> LoginFlow.command(cmd, o)
            cmd.startsWith("app.") -> app(cmd, o)
            else -> Log.w(TAG, "unknown command $cmd")
        }
    }

    private fun app(cmd: String, o: JSONObject) {
        val ctx = LimusicApp.instance
        when (cmd) {
            "app.openUrl" -> {
                val intent = Intent(Intent.ACTION_VIEW, Uri.parse(o.optString("url")))
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                try {
                    ctx.startActivity(intent)
                } catch (e: ActivityNotFoundException) {
                    Log.w(TAG, "no browser for ${o.optString("url")}")
                }
            }
            "app.share" -> {
                val send = Intent(Intent.ACTION_SEND).apply {
                    type = "text/plain"
                    putExtra(Intent.EXTRA_TEXT, o.optString("text"))
                    if (o.has("title")) putExtra(Intent.EXTRA_SUBJECT, o.optString("title"))
                }
                val chooser = Intent.createChooser(send, o.optString("title", "Share"))
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                ctx.startActivity(chooser)
            }
            "app.equalizer" -> {
                // The phone's own equalizer (Samsung SoundAlive, Dolby, the AOSP one...), bound to
                // the session both decks share, so it applies through a crossfade too.
                val intent = Intent(AudioEffect.ACTION_DISPLAY_AUDIO_EFFECT_CONTROL_PANEL).apply {
                    putExtra(AudioEffect.EXTRA_AUDIO_SESSION, PlayerEngine.audioSessionId())
                    putExtra(AudioEffect.EXTRA_PACKAGE_NAME, ctx.packageName)
                    putExtra(AudioEffect.EXTRA_CONTENT_TYPE, AudioEffect.CONTENT_TYPE_MUSIC)
                }
                val act = activity?.get()
                try {
                    if (act != null) act.startActivityForResult(intent, 0)
                    else ctx.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                    emit("app", JSONObject().put("ev", "equalizer").put("ok", true))
                } catch (e: ActivityNotFoundException) {
                    emit("app", JSONObject().put("ev", "equalizer").put("ok", false))
                }
            }
            "app.immersive" -> {
                val act = activity?.get() ?: return
                val ctl = WindowCompat.getInsetsController(act.window, act.window.decorView)
                if (o.optBoolean("on")) {
                    ctl.systemBarsBehavior =
                        WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
                    ctl.hide(WindowInsetsCompat.Type.systemBars())
                } else {
                    ctl.show(WindowInsetsCompat.Type.systemBars())
                }
            }
            "app.keepScreenOn" -> {
                val act = activity?.get() ?: return
                if (o.optBoolean("on")) {
                    act.window.addFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                } else {
                    act.window.clearFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                }
            }
            "app.statusBar" -> {
                // Light or dark status bar icons, following the app's own theme.
                val act = activity?.get() ?: return
                val ctl = WindowCompat.getInsetsController(act.window, act.window.decorView)
                val dark = o.optBoolean("dark", true)
                ctl.isAppearanceLightStatusBars = !dark
                ctl.isAppearanceLightNavigationBars = !dark
            }
            "app.bringToFront" -> {
                val intent = Intent(ctx, MainActivity::class.java)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_REORDER_TO_FRONT)
                ctx.startActivity(intent)
            }
            "app.background" -> activity?.get()?.moveTaskToBack(true)
            "app.insets" -> (activity?.get() as? MainActivity)?.pushInsets()
            "app.audioPermission" -> {
                // Local Music reads the phone's own audio files; MainActivity reports the answer.
                val perm = if (android.os.Build.VERSION.SDK_INT >= 33) {
                    android.Manifest.permission.READ_MEDIA_AUDIO
                } else {
                    android.Manifest.permission.READ_EXTERNAL_STORAGE
                }
                val granted = androidx.core.content.ContextCompat.checkSelfPermission(ctx, perm) ==
                    android.content.pm.PackageManager.PERMISSION_GRANTED
                val act = activity?.get()
                if (granted || act == null) {
                    emit("app", JSONObject().put("ev", "audioPermission").put("granted", granted)
                        .put("music", musicDir()))
                } else {
                    act.requestPermissions(arrayOf(perm), MainActivity.AUDIO_PERMISSION)
                }
            }
            else -> Log.w(TAG, "unknown app command $cmd")
        }
    }
}
