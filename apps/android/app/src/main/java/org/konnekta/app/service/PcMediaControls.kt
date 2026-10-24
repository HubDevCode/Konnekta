package org.konnekta.app.service

import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.drawable.Icon
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.R
import org.konnekta.app.core.Core
import org.konnekta.app.core.Types
import org.konnekta.app.state.Settings
import org.konnekta.app.ui.MainActivity
import java.io.File

/**
 * The computer's player on the phone, like KDE Connect: a media session per
 * computer with a MediaStyle notification, so Android shows it in the Quick
 * Settings media controls and on the lock screen — cover, title, artist,
 * previous / play-pause / next and a seek bar. The computer reports the
 * player that is playing; buttons and seeking go back as konnekta.media.request.
 * Setting "Lettore del computer nel centro di controllo" (on by default).
 */
object PcMediaControls {
    const val ACTION = "org.konnekta.app.PC_MEDIA"
    private const val TAG = "KonnektaPcMedia"
    private const val BASE_ID = 400
    private val main = Handler(Looper.getMainLooper())

    private class Entry(val deviceId: String, val session: MediaSession, val notificationId: Int) {
        var name = "Computer"
        var state = JSONObject()
        var receivedAt = 0L
        var artUrl: String? = null
        var art: Bitmap? = null
        var shown = false
    }

    private val entries = mutableMapOf<String, Entry>()

    /** Ask a computer for its player state and changes (when the setting is on). */
    fun subscribe(core: Core, deviceId: String) {
        if (!Settings.pcMediaControls.value) return
        runCatching {
            core.sendPacket(deviceId, Types.MEDIA_REQUEST,
                JSONObject().put("requestNowPlaying", true).put("requestPlayerList", true))
        }.onFailure { Log.w(TAG, "cannot ask $deviceId for its player", it) }
    }

    /** A konnekta.media packet from a computer. */
    fun onMedia(ctx: Context, core: Core, deviceId: String, deviceName: String, body: JSONObject, payloadKey: String?) {
        if (!Settings.pcMediaControls.value) return
        if (body.optBoolean("transferringAlbumArt")) {
            val url = body.optString("albumArtUrl")
            if (payloadKey != null && url.isNotEmpty()) Thread { loadArt(ctx, core, deviceId, url, payloadKey) }.start()
            return
        }
        if (body.has("playerList") && !body.has("title")) return
        main.post {
            val e = entry(ctx, deviceId)
            e.name = deviceName
            // A new track without a cover must not keep the last one.
            if (body.has("title") && !body.has("albumArtUrl")) e.state.remove("albumArtUrl")
            body.keys().forEach { k -> e.state.put(k, body.get(k)) }
            e.receivedAt = SystemClock.elapsedRealtime()
            update(ctx, e)
        }
    }

    fun remove(ctx: Context, deviceId: String) = main.post {
        entries.remove(deviceId)?.let { e ->
            ctx.getSystemService(NotificationManager::class.java).cancel(e.notificationId)
            e.session.isActive = false
            e.session.release()
        }
    }

    fun clear(ctx: Context) = main.post { entries.keys.toList().forEach { remove(ctx, it) } }

    /** A button of the notification (older Android without media controls). */
    fun onAction(deviceId: String?, action: String?) {
        val e = entries[deviceId ?: return] ?: return
        when (action) {
            "Previous", "Next", "PlayPause" -> request(e, JSONObject().put("action", action))
        }
    }

    private fun entry(ctx: Context, deviceId: String): Entry = entries.getOrPut(deviceId) {
        val s = MediaSession(ctx.applicationContext, "Konnekta $deviceId")
        val e = Entry(deviceId, s, BASE_ID + (deviceId.hashCode() and 0xff))
        s.setCallback(object : MediaSession.Callback() {
            override fun onPlay() = request(e, JSONObject().put("action", "Play"))
            override fun onPause() = request(e, JSONObject().put("action", "Pause"))
            override fun onSkipToNext() = request(e, JSONObject().put("action", "Next"))
            override fun onSkipToPrevious() = request(e, JSONObject().put("action", "Previous"))
            override fun onSeekTo(pos: Long) {
                e.state.put("pos", pos)
                e.receivedAt = SystemClock.elapsedRealtime()
                update(ctx, e)
                request(e, JSONObject().put("SetPosition", pos))
            }
        }, main)
        s.setSessionActivity(PendingIntent.getActivity(ctx, 0, Intent(ctx, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE))
        e
    }

    private fun request(e: Entry, body: JSONObject) {
        e.state.optString("player").takeIf { it.isNotEmpty() }?.let { body.put("player", it) }
        Thread { runCatching { KonnektaApp.core?.sendPacket(e.deviceId, Types.MEDIA_REQUEST, body) } }.start()
    }

    private fun update(ctx: Context, e: Entry) {
        val nm = ctx.getSystemService(NotificationManager::class.java)
        val st = e.state
        val title = st.optString("title")
        if (title.isEmpty() || !Settings.pcMediaControls.value) {
            if (e.shown) nm.cancel(e.notificationId)
            e.shown = false
            e.session.isActive = false
            return
        }
        val playing = st.optBoolean("isPlaying")
        val artist = st.optString("artist")
        val length = st.optLong("length")
        val art = e.art.takeIf { e.artUrl != null && e.artUrl == st.optString("albumArtUrl") }
        e.session.setMetadata(MediaMetadata.Builder()
            .putString(MediaMetadata.METADATA_KEY_TITLE, title)
            .putString(MediaMetadata.METADATA_KEY_ARTIST, artist)
            .putString(MediaMetadata.METADATA_KEY_ALBUM, st.optString("album"))
            .putLong(MediaMetadata.METADATA_KEY_DURATION, if (length > 0) length else -1)
            .apply { art?.let { putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, it) } }
            .build())
        var actions = PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PAUSE or PlaybackState.ACTION_PLAY_PAUSE or
            PlaybackState.ACTION_SKIP_TO_NEXT or PlaybackState.ACTION_SKIP_TO_PREVIOUS
        if (length > 0) actions = actions or PlaybackState.ACTION_SEEK_TO
        e.session.setPlaybackState(PlaybackState.Builder()
            .setActions(actions)
            // Android moves the bar on by itself from this moment.
            .setState(if (playing) PlaybackState.STATE_PLAYING else PlaybackState.STATE_PAUSED,
                st.optLong("pos"), if (playing) 1f else 0f, e.receivedAt)
            .build())
        e.session.isActive = true

        fun button(icon: Int, label: String, action: String) = Notification.Action.Builder(
            Icon.createWithResource(ctx, icon), label,
            PendingIntent.getService(ctx, e.notificationId * 4 + action.length,
                Intent(ctx, KonnektaService::class.java).setAction(ACTION).putExtra("device", e.deviceId).putExtra("action", action),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT),
        ).build()
        val n = Notification.Builder(ctx, KonnektaApp.CH_MEDIA)
            .setSmallIcon(R.drawable.ic_stat)
            .setContentTitle(title)
            .setContentText(artist.ifEmpty { e.name })
            .setSubText(e.name)
            .setLargeIcon(art)
            .setVisibility(Notification.VISIBILITY_PUBLIC)
            .setOngoing(playing)
            .setShowWhen(false)
            .setContentIntent(PendingIntent.getActivity(ctx, 0, Intent(ctx, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE))
            .addAction(button(android.R.drawable.ic_media_previous, "Precedente", "Previous"))
            .addAction(button(if (playing) android.R.drawable.ic_media_pause else android.R.drawable.ic_media_play,
                if (playing) "Pausa" else "Riproduci", "PlayPause"))
            .addAction(button(android.R.drawable.ic_media_next, "Successivo", "Next"))
            .setStyle(Notification.MediaStyle().setMediaSession(e.session.sessionToken).setShowActionsInCompactView(0, 1, 2))
            .build()
        runCatching { nm.notify(e.notificationId, n) }.onFailure { Log.w(TAG, "media notification", it) }
        e.shown = true
    }

    /** Download the cover the computer attached, scaled for the controls. */
    private fun loadArt(ctx: Context, core: Core, deviceId: String, url: String, payloadKey: String) {
        val tmp = File(ctx.cacheDir, "pc-art-${deviceId.take(8)}")
        try {
            core.call("fetchPayload", JSONObject().put("payloadKey", payloadKey).put("path", tmp.absolutePath))
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeFile(tmp.absolutePath, bounds)
            var sample = 1
            while (maxOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= 512) sample *= 2
            val bmp = BitmapFactory.decodeFile(tmp.absolutePath, BitmapFactory.Options().apply { inSampleSize = sample }) ?: return
            main.post {
                val e = entries[deviceId] ?: entry(ctx, deviceId)
                e.artUrl = url
                e.art = bmp
                if (e.state.optString("albumArtUrl") == url) update(ctx, e)
            }
        } catch (ex: Exception) {
            Log.w(TAG, "album art", ex)
        } finally {
            tmp.delete()
        }
    }
}
