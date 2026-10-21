package org.konnekta.app

import android.app.Application
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

        const val CH_SERVICE = "service"
        const val CH_ALERTS = "alerts"
        const val CH_APPROVE = "approve"
    }

    override fun onCreate() {
        super.onCreate()
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CH_SERVICE, "Connessione", NotificationManager.IMPORTANCE_MIN))
        nm.createNotificationChannel(NotificationChannel(CH_ALERTS, "Avvisi dal computer", NotificationManager.IMPORTANCE_HIGH))
        nm.createNotificationChannel(NotificationChannel(CH_APPROVE, "Approvazioni", NotificationManager.IMPORTANCE_HIGH))
    }
}
