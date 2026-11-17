package org.konnekta.app.service

import android.content.ComponentName
import android.content.Context
import android.database.ContentObserver
import android.graphics.Bitmap
import android.media.AudioManager
import android.media.MediaMetadata
import android.media.session.MediaController
import android.media.session.MediaSessionManager
import android.media.session.PlaybackState
import android.os.Handler
import android.os.Looper
import android.provider.Settings.System as SystemSettings
import android.util.Base64
import android.util.Log
import org.json.JSONArray
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.core.Types
import java.io.ByteArrayOutputStream
import java.util.concurrent.Executors

/**
 * The phone's players (Spotify, YouTube…) for the computers: state goes out
 * as `konnekta.media`, `konnekta.media.request` controls them. Android only
 * lists media sessions to an enabled notification listener.
 */
class PhoneMedia(private val ctx: Context) {
    private companion object { const val TAG = "KonnektaMedia" }
    private val main = Handler(Looper.getMainLooper())
    private val io = Executors.newSingleThreadExecutor()
    private val msm = ctx.getSystemService(MediaSessionManager::class.java)
    private val audio = ctx.getSystemService(AudioManager::class.java)
    private val listener = ComponentName(ctx, NotificationForwarder::class.java)
    private var controllers: List<MediaController> = emptyList()
    private val callbacks = mutableMapOf<MediaController, MediaController.Callback>()
    private var last = ""

    private val sessionsChanged = MediaSessionManager.OnActiveSessionsChangedListener { list -> track(list.orEmpty()) }

    /** Volume keys, other apps or a computer changed the volume: tell the computers right away. */
    private val volumeChanged = object : ContentObserver(main) {
        override fun onChange(selfChange: Boolean) = changed()
    }

    fun start() = main.post {
        runCatching { ctx.contentResolver.registerContentObserver(SystemSettings.CONTENT_URI, true, volumeChanged) }
        runCatching {
            msm.addOnActiveSessionsChangedListener(sessionsChanged, listener, main)
            track(msm.getActiveSessions(listener))
        }
        changed()
    }

    fun stop() = main.post {
        runCatching { ctx.contentResolver.unregisterContentObserver(volumeChanged) }
        runCatching { msm.removeOnActiveSessionsChangedListener(sessionsChanged) }
        track(emptyList())
        io.shutdown()
    }

    private fun track(all: List<MediaController>) {
        callbacks.forEach { (c, cb) -> c.unregisterCallback(cb) }
        callbacks.clear()
        // Our own sessions mirror the computers' players: never send them back.
        val list = all.filter { it.packageName != ctx.packageName }
        controllers = list
        for (c in list) {
            val cb = object : MediaController.Callback() {
                override fun onPlaybackStateChanged(state: PlaybackState?) = changed()
                override fun onMetadataChanged(metadata: MediaMetadata?) = changed()
            }
            c.registerCallback(cb, main)
            callbacks[c] = cb
        }
        changed()
    }

    private fun label(c: MediaController): String = runCatching {
        val pm = ctx.packageManager
        pm.getApplicationLabel(pm.getApplicationInfo(c.packageName, 0)).toString()
    }.getOrDefault(c.packageName)

    private fun find(player: String?): MediaController? =
        controllers.firstOrNull { player != null && label(it) == player }
            ?: controllers.firstOrNull { it.playbackState?.state == PlaybackState.STATE_PLAYING }
            ?: controllers.firstOrNull()

    private fun playerList() = JSONObject().put("playerList", JSONArray(controllers.map(::label)))

    /** Music volume 0-100, rounded to the nearest step. */
    private fun volume(): Int {
        val max = audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC).coerceAtLeast(1)
        return (audio.getStreamVolume(AudioManager.STREAM_MUSIC) * 100 + max / 2) / max
    }

    /** The current player, or just the volume when nothing is open. */
    private fun state(c: MediaController?): JSONObject = c?.let(::nowPlaying) ?: JSONObject().put("volume", volume())

    private fun art(c: MediaController): Bitmap? = c.metadata?.let { m ->
        m.getBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART)
            ?: m.getBitmap(MediaMetadata.METADATA_KEY_ART)
            ?: m.getBitmap(MediaMetadata.METADATA_KEY_DISPLAY_ICON)
    }

    /** Identifies the current cover: the computers ask for the picture only when it changes. */
    private fun artKey(c: MediaController): String {
        val b = art(c) ?: return ""
        val m = c.metadata
        val id = listOf(MediaMetadata.METADATA_KEY_TITLE, MediaMetadata.METADATA_KEY_ARTIST, MediaMetadata.METADATA_KEY_ALBUM)
            .joinToString("|") { m?.getString(it).orEmpty() }
        return Integer.toHexString("${label(c)}|$id|${b.width}x${b.height}".hashCode())
    }

    /** The cover as a small JPEG data URL (well under the packet size limit). */
    private fun artPacket(c: MediaController): JSONObject? {
        val b = art(c) ?: return null
        val scale = minOf(1f, 320f / maxOf(b.width, b.height).coerceAtLeast(1))
        val small = if (scale < 1f) Bitmap.createScaledBitmap(b, (b.width * scale).toInt().coerceAtLeast(1), (b.height * scale).toInt().coerceAtLeast(1), true) else b
        val out = ByteArrayOutputStream()
        if (!small.compress(Bitmap.CompressFormat.JPEG, 82, out)) return null
        return JSONObject()
            .put("player", label(c))
            .put("artKey", artKey(c))
            .put("albumArtData", "data:image/jpeg;base64," + Base64.encodeToString(out.toByteArray(), Base64.NO_WRAP))
    }

    private fun nowPlaying(c: MediaController): JSONObject {
        val m = c.metadata
        val st = c.playbackState
        val actions = st?.actions ?: 0L
        val vol = volume()
        return JSONObject()
            .put("player", label(c))
            .put("artKey", artKey(c))
            .put("title", m?.getString(MediaMetadata.METADATA_KEY_TITLE).orEmpty())
            .put("artist", m?.getString(MediaMetadata.METADATA_KEY_ARTIST).orEmpty())
            .put("album", m?.getString(MediaMetadata.METADATA_KEY_ALBUM).orEmpty())
            .put("isPlaying", st?.state == PlaybackState.STATE_PLAYING)
            .put("pos", st?.position ?: 0)
            .put("length", m?.getLong(MediaMetadata.METADATA_KEY_DURATION) ?: 0)
            .put("volume", vol)
            .put("canPause", actions and PlaybackState.ACTION_PAUSE != 0L || actions and PlaybackState.ACTION_PLAY_PAUSE != 0L)
            .put("canPlay", actions and PlaybackState.ACTION_PLAY != 0L || actions and PlaybackState.ACTION_PLAY_PAUSE != 0L)
            .put("canGoNext", actions and PlaybackState.ACTION_SKIP_TO_NEXT != 0L)
            .put("canGoPrevious", actions and PlaybackState.ACTION_SKIP_TO_PREVIOUS != 0L)
    }

    /** Send the current player when something visible changed. */
    private fun changed() {
        val body = state(find(null))
        val list = playerList()
        val key = list.toString() + JSONObject(body.toString()).apply { remove("pos") }.toString()
        if (key == last) return
        last = key
        // runCatching: a late change after stop() finds the executor shut down.
        runCatching {
            io.execute {
                for (d in connectedPaired()) {
                    send(d, list)
                    send(d, body)
                }
            }
        }
    }

    private fun send(deviceId: String, body: JSONObject) {
        runCatching { KonnektaApp.core?.sendPacket(deviceId, Types.MEDIA, body) }
    }

    /**
     * Volume 0-100 asked by a computer. A background app's setStreamVolume is
     * often ignored, so the player's media session (the system applies it)
     * goes first; the music stream is the fallback and the check.
     */
    private fun setVolume(c: MediaController?, percent: Int) {
        val max = audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC).coerceAtLeast(1)
        val target = (percent * max + 50) / 100
        val viaSession = c?.playbackInfo?.let { pi ->
            runCatching {
                c.setVolumeTo((percent * pi.maxVolume + 50) / 100, 0)
                true
            }.onFailure { Log.w(TAG, "session volume refused", it) }.getOrDefault(false)
        } ?: false
        // Cast and other remote players have their own volume: the session is all there is.
        if (viaSession && c?.playbackInfo?.playbackType == MediaController.PlaybackInfo.PLAYBACK_TYPE_REMOTE) {
            changed()
            return
        }
        if (!viaSession || audio.getStreamVolume(AudioManager.STREAM_MUSIC) != target) {
            runCatching { audio.setStreamVolume(AudioManager.STREAM_MUSIC, target, 0) }
                .onFailure { Log.w(TAG, "stream volume refused", it) }
        }
        // Step by step as a last resort (some phones accept only adjustments).
        var guard = max + 1
        while (guard-- > 0) {
            val now = audio.getStreamVolume(AudioManager.STREAM_MUSIC)
            if (now == target) break
            val dir = if (now < target) AudioManager.ADJUST_RAISE else AudioManager.ADJUST_LOWER
            if (runCatching { audio.adjustStreamVolume(AudioManager.STREAM_MUSIC, dir, 0) }.isFailure) break
            if (audio.getStreamVolume(AudioManager.STREAM_MUSIC) == now) break
        }
        Log.d(TAG, "volume $percent% -> ${audio.getStreamVolume(AudioManager.STREAM_MUSIC)}/$max (session=$viaSession)")
        changed()
    }

    /** A computer controls the phone's media. */
    fun handle(deviceId: String, b: JSONObject) = main.post {
        val player = b.optString("player").ifEmpty { null }
        val c = find(player)
        val t = c?.transportControls
        when (b.optString("action")) {
            "Play" -> t?.play()
            "Pause" -> t?.pause()
            "PlayPause" -> if (c?.playbackState?.state == PlaybackState.STATE_PLAYING) t?.pause() else t?.play()
            "Stop" -> t?.stop()
            "Next" -> t?.skipToNext()
            "Previous" -> t?.skipToPrevious()
        }
        if (b.has("setVolume")) setVolume(c, b.optInt("setVolume").coerceIn(0, 100))
        if (b.has("SetPosition")) t?.seekTo(b.optLong("SetPosition").coerceAtLeast(0))
        if (b.has("Seek")) c?.playbackState?.let { st -> t?.seekTo((st.position + b.optLong("Seek") / 1000).coerceAtLeast(0)) }
        val list = playerList()
        val now = state(c)
        val wantArt = b.optBoolean("requestAlbumArt")
        runCatching {
            io.execute {
                if (b.optBoolean("requestPlayerList")) send(deviceId, list)
                if (b.optBoolean("requestNowPlaying") || b.optBoolean("requestVolume") || b.has("action")) send(deviceId, now)
                if (wantArt) c?.let { runCatching { artPacket(it) }.getOrNull() }?.let { send(deviceId, it) }
            }
        }
    }
}
