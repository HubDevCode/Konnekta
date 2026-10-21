package org.konnekta.app.service

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.media.VolumeProvider
import android.media.session.MediaSession
import android.media.session.PlaybackState
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import kotlinx.coroutines.flow.MutableStateFlow
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.R
import org.konnekta.app.core.Types

/**
 * Volume keys change slide also with the screen off or another app open
 * (setting "Tasti del volume ovunque"). A media session with a *remote*
 * volume takes the volume keys away from the phone's own volume: up = next
 * slide, down = previous. It stays on until stopped from the Presentation
 * screen or its notification. Most phones route the keys to it with the
 * screen off; some vendors may not.
 */
object PresenterRemote {
    const val ACTION_STOP = "org.konnekta.app.PRESENTER_STOP"
    private const val NOTIFICATION = 30

    /** The computer the keys drive while active. */
    val active = MutableStateFlow<String?>(null)
    private var session: MediaSession? = null

    fun start(ctx: Context, deviceId: String, deviceName: String) {
        val app = ctx.applicationContext
        stop(app)
        val s = MediaSession(app, "Konnekta presentazione")
        s.setPlaybackToRemote(object : VolumeProvider(VOLUME_CONTROL_RELATIVE, 100, 50) {
            override fun onAdjustVolume(direction: Int) {
                when {
                    direction > 0 -> send(deviceId, "next")
                    direction < 0 -> send(deviceId, "previous")
                }
            }
        })
        // Android sends the keys to the session that "plays".
        s.setPlaybackState(PlaybackState.Builder().setState(PlaybackState.STATE_PLAYING, 0, 1f).build())
        s.isActive = true
        session = s
        active.value = deviceId
        val stop = PendingIntent.getService(app, NOTIFICATION, Intent(app, KonnektaService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val n = NotificationCompat.Builder(app, KonnektaApp.CH_SERVICE)
            .setSmallIcon(R.drawable.ic_stat)
            .setContentTitle("Presentazione su $deviceName")
            .setContentText("Volume su = avanti, giù = indietro, anche a schermo spento")
            .setOngoing(true)
            .setSilent(true)
            .addAction(0, "Ferma", stop)
            .build()
        runCatching { NotificationManagerCompat.from(app).notify(NOTIFICATION, n) }
    }

    fun stop(ctx: Context) {
        session?.let { runCatching { it.isActive = false; it.release() } }
        session = null
        active.value = null
        runCatching { NotificationManagerCompat.from(ctx.applicationContext).cancel(NOTIFICATION) }
    }

    private fun send(deviceId: String, cmd: String) {
        Thread { runCatching { KonnektaApp.core?.sendPacket(deviceId, Types.PRESENTER, JSONObject().put("konnektaKey", cmd)) } }.start()
    }
}
