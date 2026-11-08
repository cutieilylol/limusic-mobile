package com.limusic.mobile

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import org.json.JSONObject

/**
 * Where Discord's sign-in page sends you back (`limusicdiscord://oauth2/callback?code=…&state=…`).
 * Hands the answer to the Rust side (mobile/discord_net.rs), which checks the state and trades the
 * code for a token, and brings Limusic back to the front. Never shows anything itself.
 */
class DiscordOAuthActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        handle(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handle(intent)
    }

    private fun handle(intent: Intent?) {
        val uri = intent?.data
        if (uri != null) {
            LimusicBridge.emit(
                "discord",
                JSONObject()
                    .put("ev", "callback")
                    .put("code", uri.getQueryParameter("code") ?: "")
                    .put("state", uri.getQueryParameter("state") ?: "")
                    .put("error", uri.getQueryParameter("error") ?: ""),
            )
        }
        startActivity(
            Intent(this, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_REORDER_TO_FRONT or Intent.FLAG_ACTIVITY_NEW_TASK),
        )
        finish()
    }
}
