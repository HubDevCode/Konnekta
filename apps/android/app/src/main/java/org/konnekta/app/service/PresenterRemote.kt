package org.konnekta.app.service

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioTrack
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import kotlinx.coroutines.flow.MutableStateFlow
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.R
import org.konnekta.app.core.Types

/**
 * Volume keys change slide also with the screen off or another app open
 * (setting "Tasti del volume ovunque").
 *
 * Android gives the volume keys to the media volume while media plays, also
 * with the screen off. So while active, Konnekta plays silence, keeps the
 * media volume in the middle, and turns every change into a slide: up =
 * next, down = previous, then puts the volume back. (A remote-volume media
 * session would be cleaner, but since Android 12 the system only routes the
 * keys to it while casting.) The media volume is restored when stopped.
 */
object PresenterRemote {
    const val ACTION_STOP = "org.konnekta.app.PRESENTER_STOP"
    private const val NOTIFICATION = 30
    private const val TAG = "KonnektaPresenter"
    private const val VOLUME_CHANGED = "android.media.VOLUME_CHANGED_ACTION"
    private const val EXTRA_STREAM = "android.media.EXTRA_VOLUME_STREAM_TYPE"
    private const val EXTRA_VALUE = "android.media.EXTRA_VOLUME_STREAM_VALUE"

    /** The computer the keys drive while active. */
    val active = MutableStateFlow<String?>(null)
    private var track: AudioTrack? = null
    private var receiver: BroadcastReceiver? = null
    private var savedVolume = -1
    private var middle = 0

    fun start(ctx: Context, deviceId: String, deviceName: String) {
        val app = ctx.applicationContext
        stop(app)
        val am = app.getSystemService(AudioManager::class.java)
        val max = am.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
        middle = (max / 2).coerceAtLeast(1)
        savedVolume = am.getStreamVolume(AudioManager.STREAM_MUSIC)
        am.setStreamVolume(AudioManager.STREAM_MUSIC, middle, 0)
        track = silence()
        receiver = object : BroadcastReceiver() {
            override fun onReceive(c: Context, i: Intent) {
                if (i.getIntExtra(EXTRA_STREAM, -1) != AudioManager.STREAM_MUSIC) return
                val v = i.getIntExtra(EXTRA_VALUE, middle)
                if (v == middle) return
                send(deviceId, if (v > middle) "next" else "previous")
                // Back to the middle so the next press is seen too.
                am.setStreamVolume(AudioManager.STREAM_MUSIC, middle, 0)
            }
        }.also { // A protected system broadcast: only Android sends it.
            ContextCompat.registerReceiver(app, it, IntentFilter(VOLUME_CHANGED), ContextCompat.RECEIVER_EXPORTED) }
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
        val app = ctx.applicationContext
        receiver?.let { runCatching { app.unregisterReceiver(it) } }
        receiver = null
        track?.let { runCatching { it.stop() }; it.release() }
        track = null
        if (savedVolume >= 0) {
            runCatching { app.getSystemService(AudioManager::class.java).setStreamVolume(AudioManager.STREAM_MUSIC, savedVolume, 0) }
            savedVolume = -1
        }
        active.value = null
        runCatching { NotificationManagerCompat.from(app).cancel(NOTIFICATION) }
    }

    /** One second of silence on loop, as media (no audio focus taken). */
    private fun silence(): AudioTrack? = runCatching {
        val rate = 8000
        val frames = rate
        val t = AudioTrack.Builder()
            .setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_UNKNOWN).build())
            .setAudioFormat(AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                .setSampleRate(rate).setChannelMask(AudioFormat.CHANNEL_OUT_MONO).build())
            .setTransferMode(AudioTrack.MODE_STATIC)
            .setBufferSizeInBytes(frames * 2)
            .build()
        t.write(ShortArray(frames), 0, frames)
        t.setLoopPoints(0, frames, -1)
        t.play()
        t
    }.onFailure { Log.w(TAG, "silent track failed", it) }.getOrNull()

    private fun send(deviceId: String, cmd: String) {
        Thread { runCatching { KonnektaApp.core?.sendPacket(deviceId, Types.PRESENTER, JSONObject().put("konnektaKey", cmd)) } }.start()
    }
}
