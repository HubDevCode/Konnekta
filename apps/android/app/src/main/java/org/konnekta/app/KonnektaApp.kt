package org.konnekta.app

import android.app.Activity
import android.app.Application
import android.os.Bundle
import android.app.NotificationChannel
import android.app.NotificationManager
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import org.json.JSONObject
import org.konnekta.app.core.Core

class KonnektaApp : Application() {
    companion object {
        /** Set by [org.konnekta.app.service.KonnektaService] while it runs. */
        @Volatile
        var core: Core? = null

        private val _events = MutableSharedFlow<JSONObject>(extraBufferCapacity = 64)
        /** Core events for the UI. */
        val events: SharedFlow<JSONObject> = _events

        fun publish(e: JSONObject) {
            _events.tryEmit(e)
        }

        /** An activity of the app is on screen. */
        @Volatile
        var uiVisible = false
            private set

        /** Called on the main thread when [uiVisible] changes. */
        @Volatile
        var onUiVisible: (() -> Unit)? = null

        const val CH_SERVICE = "service"
        const val CH_ALERTS = "alerts"
        const val CH_APPROVE = "approve"
        const val CH_MEDIA = "pcmedia"
    }

    override fun onCreate() {
        super.onCreate()
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.O) {
            val nm = getSystemService(NotificationManager::class.java)
            nm.createNotificationChannel(NotificationChannel(CH_SERVICE, "Connessione", NotificationManager.IMPORTANCE_MIN))
            nm.createNotificationChannel(NotificationChannel(CH_ALERTS, "Avvisi dal computer", NotificationManager.IMPORTANCE_HIGH))
            nm.createNotificationChannel(NotificationChannel(CH_APPROVE, "Approvazioni", NotificationManager.IMPORTANCE_HIGH))
            nm.createNotificationChannel(NotificationChannel(CH_MEDIA, "Lettore del computer", NotificationManager.IMPORTANCE_LOW))
        }
        registerActivityLifecycleCallbacks(object : ActivityLifecycleCallbacks {
            private var started = 0
            override fun onActivityStarted(a: Activity) {
                if (started++ == 0) { uiVisible = true; onUiVisible?.invoke() }
            }
            override fun onActivityStopped(a: Activity) {
                if (--started <= 0) { started = 0; uiVisible = false; onUiVisible?.invoke() }
            }
            override fun onActivityCreated(a: Activity, b: Bundle?) {}
            override fun onActivityResumed(a: Activity) {}
            override fun onActivityPaused(a: Activity) {}
            override fun onActivitySaveInstanceState(a: Activity, b: Bundle) {}
            override fun onActivityDestroyed(a: Activity) {}
        })
        // The service may start without the UI (boot, after an update).
        org.konnekta.app.state.Settings.load(this)
        org.konnekta.app.service.ClipboardSync.install(this)
    }
}
