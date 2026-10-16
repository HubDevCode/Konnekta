package org.konnekta.konnekta

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder

/** Keeps connections alive while the app is in the background. */
class KonnektaForegroundService : Service() {
    companion object {
        const val CHANNEL = "konnekta_service"
        const val EXTRA_TYPES = "types"
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val nm = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= 26) {
            nm.createNotificationChannel(NotificationChannel(CHANNEL, "Konnekta", NotificationManager.IMPORTANCE_LOW))
        }
        val n: Notification = Notification.Builder(this, CHANNEL)
            .setContentTitle("Konnekta")
            .setContentText("Connesso ai tuoi dispositivi")
            .setSmallIcon(android.R.drawable.stat_sys_data_bluetooth)
            .setOngoing(true)
            .build()
        val types = intent?.getIntExtra(EXTRA_TYPES, 0) ?: 0
        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(1, n, if (types != 0) types else ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } else {
            startForeground(1, n)
        }
        return START_STICKY
    }
}
