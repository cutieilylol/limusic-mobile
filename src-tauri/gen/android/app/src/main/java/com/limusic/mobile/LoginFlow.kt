package com.limusic.mobile

import android.annotation.SuppressLint
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.ViewGroup
import android.webkit.CookieManager
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.FrameLayout
import androidx.activity.OnBackPressedCallback
import androidx.appcompat.app.AppCompatActivity
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.webkit.WebSettingsCompat
import androidx.webkit.WebViewFeature
import org.json.JSONObject
import java.lang.ref.WeakReference

/**
 * Google sign-in, the Android half of `src-tauri/src/mobile/session.rs`.
 *
 * [LoginActivity] is the desktop login window as a full screen: Google's own sign-in page in a
 * WebView, and once it lands on music.youtube.com the youtube.com cookie jar goes to Rust. The
 * session refresh is the desktop's hidden refresh window: music.youtube.com loaded off screen so
 * Google can roll the short-lived cookies, then the jar is read again.
 *
 * Both share the process-wide [CookieManager], which persists, so the Google session survives
 * restarts exactly like the desktop login webview's profile.
 */
object LoginFlow {
    private val main = Handler(Looper.getMainLooper())
    var current: WeakReference<LoginActivity>? = null
    private var refresher: WebView? = null

    fun command(cmd: String, o: JSONObject) {
        when (cmd) {
            "login.open" -> {
                val ctx = LimusicApp.instance
                if (o.optBoolean("clear")) {
                    CookieManager.getInstance().removeAllCookies(null)
                    CookieManager.getInstance().flush()
                }
                val intent = Intent(ctx, LoginActivity::class.java)
                    .putExtra("url", o.optString("url"))
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                ctx.startActivity(intent)
            }
            "login.close" -> current?.get()?.finishQuietly()
            "login.refresh" -> refresh(o.optString("url"))
            "login.refreshDone" -> {
                refresher?.destroy()
                refresher = null
            }
        }
    }

    fun youtubeCookies(): String =
        CookieManager.getInstance().getCookie("https://music.youtube.com") ?: ""

    /** Google's login page refuses to load in anything it recognises as an embedded WebView, and
     *  the tell it reads is the `; wv` token, so the WebView passes for the phone's Chrome. */
    fun userAgent(wv: WebView): String =
        wv.settings.userAgentString.replace("; wv)", ")").replace(Regex("Version/\\d+\\.\\d+ "), "")

    /** The other tell: the `X-Requested-With: <package>` header a WebView adds. Sent to nobody. */
    fun hideWebView(wv: WebView) {
        wv.settings.userAgentString = userAgent(wv)
        if (WebViewFeature.isFeatureSupported(WebViewFeature.REQUESTED_WITH_HEADER_ALLOW_LIST)) {
            WebSettingsCompat.setRequestedWithHeaderOriginAllowList(wv.settings, emptySet())
        }
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun refresh(url: String) {
        refresher?.destroy()
        val wv = WebView(LimusicApp.instance)
        wv.settings.javaScriptEnabled = true
        wv.settings.domStorageEnabled = true
        hideWebView(wv)
        CookieManager.getInstance().setAcceptThirdPartyCookies(wv, true)
        wv.webViewClient = object : WebViewClient() {
            override fun onPageFinished(view: WebView, url: String) {
                if (Uri.parse(url).host != "music.youtube.com") return
                // The rolled cookies can land a moment after the page; offer the jar a few times
                // and let the Rust side pick the first one that changed (session.rs).
                for (i in 0 until 8) {
                    main.postDelayed({
                        if (refresher === view) {
                            LimusicBridge.emit(
                                "login", JSONObject().put("ev", "refresh").put("cookie", youtubeCookies())
                            )
                        }
                    }, 500L * i)
                }
            }
        }
        refresher = wv
        wv.loadUrl(url)
    }
}

class LoginActivity : AppCompatActivity() {
    private var reported = false
    private lateinit var web: WebView

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        LoginFlow.current = WeakReference(this)
        web = WebView(this)
        web.settings.javaScriptEnabled = true
        web.settings.domStorageEnabled = true
        LoginFlow.hideWebView(web)
        CookieManager.getInstance().setAcceptCookie(true)
        CookieManager.getInstance().setAcceptThirdPartyCookies(web, true)
        web.webViewClient = object : WebViewClient() {
            override fun onPageFinished(view: WebView, url: String) {
                if (Uri.parse(url).host == "music.youtube.com") pollCookies(0)
            }
        }
        val root = FrameLayout(this)
        root.addView(web, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        ViewCompat.setOnApplyWindowInsetsListener(root) { v, insets ->
            val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.ime())
            v.setPadding(bars.left, bars.top, bars.right, bars.bottom)
            WindowInsetsCompat.CONSUMED
        }
        setContentView(root)
        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {
                if (web.canGoBack()) web.goBack() else finish()
            }
        })
        web.loadUrl(intent.getStringExtra("url") ?: "https://accounts.google.com/")
    }

    /** The session cookies can land a moment after music.youtube.com finishes loading, so the jar
     *  is read a few times (the desktop login does the same, session.rs). */
    private fun pollCookies(attempt: Int) {
        if (reported || isFinishing) return
        CookieManager.getInstance().flush()
        val cookie = LoginFlow.youtubeCookies()
        if (cookie.contains("SAPISID=")) {
            reported = true
            LimusicBridge.emit("login", JSONObject().put("ev", "cookies").put("cookie", cookie))
        } else if (attempt < 8) {
            web.postDelayed({ pollCookies(attempt + 1) }, 500)
        }
    }

    fun finishQuietly() {
        reported = true
        finish()
    }

    override fun onDestroy() {
        if (!reported) LimusicBridge.emit("login", JSONObject().put("ev", "closed"))
        if (LoginFlow.current?.get() === this) LoginFlow.current = null
        web.destroy()
        super.onDestroy()
    }
}
