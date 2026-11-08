package com.limusic.mobile

import android.app.Application

/**
 * Loads the Rust library before anything else runs and hands it the JNI bridge, so commands the
 * Rust setup sends while the first activity is still coming up have somewhere to go.
 */
class LimusicApp : Application() {
    override fun onCreate() {
        super.onCreate()
        instance = this
        // Same library WryActivity's Rust object loads; a second load is a no-op.
        System.loadLibrary("app_lib")
        LimusicBridge.nativeInit()
    }

    companion object {
        lateinit var instance: LimusicApp
            private set
    }
}
