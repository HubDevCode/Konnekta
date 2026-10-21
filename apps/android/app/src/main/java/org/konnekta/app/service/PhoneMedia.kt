package org.konnekta.app.service

import android.content.ComponentName
import android.content.Context
import android.media.AudioManager
import android.media.MediaMetadata
import android.media.session.MediaController
import android.media.session.MediaSessionManager
import android.media.session.PlaybackState
import android.os.Handler
import android.os.Looper
import org.json.JSONArray
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.core.Types
import java.util.concurrent.Executors

/**
 * The phone's players (Spotify, YouTube…) for the computers: state goes out
 * as `konnekta.media`, `konnekta.media.request` controls them. Android only
 * lists media sessions to an enabled notification listener.
 */
class PhoneMedia(private val ctx: Context) {
    private val main = Handler(Looper.getMainLooper())
    private val io = Executors.newSingleThreadExecutor()
    private val msm = ctx.getSystemService(MediaSessionManager::class.java)
    private val audio = ctx.getSystemService(AudioManager::class.java)
    private val listener = ComponentName(ctx, NotificationForwarder::class.java)
    private var controllers: List<MediaController> = emptyList()
    private val callbacks = mutableMapOf<MediaController, MediaController.Callback>()
    private var last = ""

    private val sessionsChanged = MediaSessionManager.OnActiveSessionsChangedListener { list -> track(list.orEmpty()) }

    fun start() = main.post {
        runCatching {
            msm.addOnActiveSessionsChangedListener(sessionsChanged, listener, main)
            track(msm.getActiveSessions(listener))
        }
    }

    fun stop() = main.post {
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

    private fun nowPlaying(c: MediaController): JSONObject {
        val m = c.metadata
        val st = c.playbackState
        val actions = st?.actions ?: 0L
        val vol = audio.getStreamVolume(AudioManager.STREAM_MUSIC) * 100 /
            audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC).coerceAtLeast(1)
        return JSONObject()
            .put("player", label(c))
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
        val c = find(null)
        val body = c?.let(::nowPlaying)
        val list = playerList()
        val key = list.toString() + (body?.let { JSONObject(it.toString()).apply { remove("pos") }.toString() } ?: "")
        if (key == last) return
        last = key
        io.execute {
            for (d in connectedPaired()) {
                send(d, list)
                body?.let { send(d, it) }
            }
        }
    }

    private fun send(deviceId: String, body: JSONObject) {
        runCatching { KonnektaApp.core?.sendPacket(deviceId, Types.MEDIA, body) }
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
        if (b.has("setVolume")) {
            val max = audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
            audio.setStreamVolume(AudioManager.STREAM_MUSIC, (b.optInt("setVolume").coerceIn(0, 100) * max / 100), 0)
        }
        if (b.has("SetPosition")) t?.seekTo(b.optLong("SetPosition").coerceAtLeast(0))
        if (b.has("Seek")) c?.playbackState?.let { st -> t?.seekTo((st.position + b.optLong("Seek") / 1000).coerceAtLeast(0)) }
        val list = playerList()
        val now = c?.let(::nowPlaying)
        io.execute {
            if (b.optBoolean("requestPlayerList")) send(deviceId, list)
            if (b.optBoolean("requestNowPlaying") || b.optBoolean("requestVolume") || b.has("action")) now?.let { send(deviceId, it) }
        }
    }
}
