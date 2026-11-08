package com.limusic.mobile

import android.annotation.SuppressLint
import android.webkit.WebView
import android.webkit.WebViewClient
import org.json.JSONObject

/**
 * Off-screen WebViews for the Rust side's JS harness (`src-tauri/src/mobile/webview.rs`): the
 * cipher deobfuscator runs YouTube's own player code in one. On desktop that is a hidden Tauri
 * window; Tauri gives Android exactly one webview, the UI, so these are plain Android WebViews
 * that are never attached to a window.
 */
object HiddenWebViews {
    private val views = HashMap<String, WebView>()

    fun command(cmd: String, o: JSONObject) {
        val label = o.optString("label")
        when (cmd) {
            "webview.create" -> create(label, o.optString("html"))
            "webview.eval" -> views[label]?.evaluateJavascript(o.optString("js"), null) ?: gone(label)
            "webview.evalJson" -> {
                val id = o.optLong("id")
                val wv = views[label]
                if (wv == null) {
                    gone(label)
                    result(id, "null")
                } else {
                    wv.evaluateJavascript(o.optString("js")) { json -> result(id, json ?: "null") }
                }
            }
            "webview.destroy" -> destroy(label)
        }
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun create(label: String, html: String) {
        destroy(label)
        val wv = WebView(LimusicApp.instance)
        wv.settings.javaScriptEnabled = true
        wv.settings.domStorageEnabled = true
        wv.settings.blockNetworkImage = true
        wv.webViewClient = WebViewClient()
        wv.loadDataWithBaseURL("https://localhost/", html, "text/html", "utf-8", null)
        views[label] = wv
    }

    private fun destroy(label: String) {
        views.remove(label)?.let {
            it.stopLoading()
            it.destroy()
        }
    }

    private fun result(id: Long, json: String) {
        LimusicBridge.emit("webview", JSONObject().put("ev", "result").put("id", id).put("json", json))
    }

    private fun gone(label: String) {
        LimusicBridge.emit("webview", JSONObject().put("ev", "gone").put("label", label))
    }
}
