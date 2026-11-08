package com.limusic.mobile

import android.Manifest
import android.content.ComponentName
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.ViewGroup
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge
import androidx.core.content.ContextCompat
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import com.google.common.util.concurrent.ListenableFuture
import java.lang.ref.WeakReference

class MainActivity : TauriActivity() {
    // Back is ours: the UI closes its own sheets first, then history, then the app goes to the
    // background instead of finishing (the music keeps playing either way, but a finished
    // activity would cost a cold start of the UI next time).
    override val handleBackNavigation: Boolean = false

    private var web: WebView? = null
    private var controller: ListenableFuture<MediaController>? = null
    private val main = Handler(Looper.getMainLooper())
    private var insetsJs = ""
    private val backCallback = object : OnBackPressedCallback(true) {
        override fun handleOnBackPressed() = back()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        // chrome://inspect over USB only for a debug build.
        WebView.setWebContentsDebuggingEnabled(BuildConfig.DEBUG)
        super.onCreate(savedInstanceState)
        LimusicBridge.activity = WeakReference(this)
        // A second activity in a process that is still running (the first was swiped away from
        // Recents while the music kept playing): the Rust core is up, but its window went with the
        // old activity, so it has to build the UI again for this one.
        if (everCreated) {
            LimusicBridge.emit("app", org.json.JSONObject().put("ev", "activityRecreated"))
        }
        everCreated = true
        onBackPressedDispatcher.addCallback(this, backCallback)
        if (Build.VERSION.SDK_INT >= 33 &&
            ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
    }

    override fun onStart() {
        super.onStart()
        // Binding a controller is what brings the playback service (and its media session) up.
        if (controller == null) {
            val token = SessionToken(this, ComponentName(this, PlaybackService::class.java))
            controller = MediaController.Builder(this, token).buildAsync()
        }
    }

    override fun onResume() {
        super.onResume()
        LimusicBridge.activity = WeakReference(this)
        pushInsets()
    }

    override fun onDestroy() {
        controller?.let { MediaController.releaseFuture(it) }
        controller = null
        super.onDestroy()
    }

    override fun onWebViewCreate(webView: WebView) {
        web = webView
        // Back callbacks run newest first, and the webview's own history handler can be added
        // after onCreate: re-adding ours here keeps it in front, so the UI always gets the first say.
        backCallback.remove()
        onBackPressedDispatcher.addCallback(this, backCallback)
        // Edge to edge: the page draws under the status and navigation bars and pads itself with
        // these (`--safe-*` in layout.css). The keyboard instead shrinks the webview, so a focused
        // field and the sheet or dialog it sits in stay above it. A margin, not padding: a WebView
        // lays its page out over its whole box whatever its padding says, so padding left the
        // viewport full height with the keyboard over its bottom half.
        ViewCompat.setOnApplyWindowInsetsListener(webView) { v, insets ->
            val bars = insets.getInsets(
                WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
            )
            val ime = insets.getInsets(WindowInsetsCompat.Type.ime())
            val d = resources.displayMetrics.density
            val bottom = if (ime.bottom > 0) 0 else bars.bottom
            (v.layoutParams as? ViewGroup.MarginLayoutParams)?.let { lp ->
                if (lp.bottomMargin != ime.bottom) {
                    lp.bottomMargin = ime.bottom
                    v.layoutParams = lp
                }
            }
            insetsJs = "(function(s){s.setProperty('--safe-top','${bars.top / d}px');" +
                "s.setProperty('--safe-bottom','${bottom / d}px');" +
                "s.setProperty('--safe-left','${bars.left / d}px');" +
                "s.setProperty('--safe-right','${bars.right / d}px');" +
                "document.documentElement.classList.add('phone');" +
                "document.documentElement.classList.toggle('keyboard-open',${ime.bottom > 0});})" +
                "(document.documentElement.style);"
            pushInsets()
            insets
        }
        // The SPA keeps one document for its whole life, so the values set on it stick; these
        // retries only cover the page still loading when the first insets arrive.
        for (ms in longArrayOf(300, 1000, 2500, 6000)) main.postDelayed({ pushInsets() }, ms)
    }

    override fun onRequestPermissionsResult(code: Int, perms: Array<out String>, results: IntArray) {
        super.onRequestPermissionsResult(code, perms, results)
        if (code == AUDIO_PERMISSION) {
            val granted = results.isNotEmpty() && results[0] == PackageManager.PERMISSION_GRANTED
            LimusicBridge.emit(
                "app",
                org.json.JSONObject().put("ev", "audioPermission").put("granted", granted)
                    .put("music", LimusicBridge.musicDir()),
            )
        }
    }

    companion object {
        const val AUDIO_PERMISSION = 2
        private var everCreated = false
    }

    fun pushInsets() {
        if (insetsJs.isNotEmpty()) web?.evaluateJavascript(insetsJs, null)
    }

    private fun back() {
        val wv = web ?: run { moveTaskToBack(true); return }
        wv.evaluateJavascript("(window.__limusicBack&&window.__limusicBack())===true") { handled ->
            if (handled == "true") return@evaluateJavascript
            if (wv.canGoBack()) wv.goBack() else moveTaskToBack(true)
        }
    }
}
