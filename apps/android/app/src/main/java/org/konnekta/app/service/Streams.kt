package org.konnekta.app.service

import android.annotation.SuppressLint
import android.content.Context
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.AudioTrack
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.media.MediaRecorder
import android.media.projection.MediaProjection
import android.os.Handler
import android.os.HandlerThread
import android.util.Log
import android.util.Range
import android.view.Surface
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.core.Core
import org.konnekta.app.core.Frame

/** A computer asks the phone to produce a stream; the user must accept. */
data class StreamRequest(val deviceId: String, val deviceName: String, val plugin: String, val session: String, val params: JSONObject) {
    val key get() = "$plugin/$deviceId"
}

/** A stream running now (either direction). */
data class LiveStream(val plugin: String, val deviceId: String, val session: String, val outgoing: Boolean) {
    val key get() = "$plugin/$deviceId"
}

/**
 * Streams between phone and computer. The phone produces microphone
 * (PCM s16le 48 kHz mono), camera and screen (H.264 Annex-B); it consumes
 * the computer's screen (H.264) and audio (PCM s16le 48 kHz stereo).
 * Channels are opened by the core (TLS, permission-checked); here only
 * capture, encoding, decoding and playback.
 */
object Streams {
    const val TAG = "Konnekta"
    val TYPES = mapOf(
        "microphone" to "konnekta.microphone",
        "camera" to "konnekta.camera",
        "phonescreen" to "konnekta.phonescreen",
        "desktop" to "konnekta.desktop",
        "systemaudio" to "konnekta.systemaudio",
        "phoneaudio" to "konnekta.phoneaudio",
    )

    /** Streams the phone produces (the computer consumes them). */
    val PRODUCED = setOf("microphone", "camera", "phonescreen", "phoneaudio")

    /** These need the screen-capture consent (MediaProjection). */
    val NEEDS_PROJECTION = setOf("phonescreen", "phoneaudio")

    fun newRequest(deviceId: String, deviceName: String, plugin: String, params: JSONObject = JSONObject()) =
        StreamRequest(deviceId, deviceName, plugin, newSession(), params)

    val pending = MutableStateFlow<List<StreamRequest>>(emptyList())

    /** A stream the computer refused because something is not installed there. */
    data class StreamProblem(val deviceName: String, val plugin: String, val missing: List<String>, val fix: List<String>)
    val problem = MutableStateFlow<StreamProblem?>(null)
    val live = MutableStateFlow<Map<String, LiveStream>>(emptyMap())

    /** Called when a request was accepted or declined (the service clears its notification). */
    @Volatile
    var onRequestsChanged: (() -> Unit)? = null

    /** Called after a stream started or ended (the service adjusts its foreground types). */
    @Volatile
    var onChanged: (() -> Unit)? = null

    /** A producer whose computer never connects gives up after this long. */
    private const val CONNECT_TIMEOUT_MS = 15_000L

    /** Remote desktop: the decoder draws here; set by the screen showing it. */
    @Volatile
    var desktopSurface: Surface? = null
        set(v) {
            field = v
            desktopDecoders.values.forEach { it.surfaceChanged() }
        }
    /** Webcam page: the camera also draws here while it streams. */
    @Volatile
    var cameraPreview: Surface? = null

    /** Size of the computer's screen as decoded (for the touch mapping). */
    val desktopSize = MutableStateFlow<Pair<Int, Int>?>(null)

    private val workers = mutableMapOf<String, Worker>()
    /** Devices whose phone audio rides on the phone-screen capture. */
    private val sharedAudio = java.util.Collections.synchronizedSet(mutableSetOf<String>())
    private val desktopDecoders = mutableMapOf<String, DesktopDecoder>()

    interface Worker {
        fun stop()
    }

    private fun send(deviceId: String, plugin: String, body: JSONObject) {
        runCatching { KonnektaApp.core?.sendPacket(deviceId, TYPES.getValue(plugin), body) }
    }

    private fun newSession() = java.lang.Long.toHexString(System.nanoTime())

    // --- packets from the computer -------------------------------------------

    fun onPacket(ctx: Context, core: Core, deviceId: String, deviceName: String, plugin: String, b: JSONObject): StreamRequest? {
        val session = b.optString("session")
        val params = b.optJSONObject("params") ?: JSONObject()
        when (b.optString("action")) {
            "stop" -> {
                pending.update { l -> l.filterNot { it.deviceId == deviceId && it.plugin == plugin } }
                stop(deviceId, plugin, notifyPeer = false)
                // The computer lacks a program or module: tell the user how to fix it.
                val missing = b.optJSONArray("missing")
                if (missing != null && missing.length() > 0) {
                    problem.value = StreamProblem(
                        deviceName, plugin,
                        (0 until missing.length()).map { missing.optString(it) },
                        b.optJSONArray("fix")?.let { f -> (0 until f.length()).map { f.optString(it) } }.orEmpty(),
                    )
                }
            }
            "request" -> if (plugin in PRODUCED) {
                val r = StreamRequest(deviceId, deviceName, plugin, session, params)
                pending.update { l -> l.filterNot { it.key == r.key } + r }
                return r
            }
            "offer" -> if (plugin in setOf("desktop", "systemaudio")) consume(core, deviceId, plugin, session, b)
        }
        return null
    }

    fun decline(r: StreamRequest) {
        pending.update { l -> l.filterNot { it.key == r.key } }
        onRequestsChanged?.invoke()
        Thread { send(r.deviceId, r.plugin, JSONObject().put("action", "stop").put("session", r.session).put("reason", "rifiutato sul telefono")) }.start()
    }

    fun stop(deviceId: String, plugin: String, notifyPeer: Boolean = true) {
        val key = "$plugin/$deviceId"
        // The screen's own audio shares its capture: they end together.
        if (plugin == "phonescreen" && sharedAudio.remove(deviceId)) stop(deviceId, "phoneaudio", notifyPeer)
        val l = live.value[key]
        synchronized(workers) { workers.remove(key) }?.stop()
        desktopDecoders.remove(key)
        live.update { it - key }
        if (plugin == "desktop") desktopSize.value = null
        if (l != null) onChanged?.invoke()
        if (notifyPeer && l != null) Thread { send(deviceId, plugin, JSONObject().put("action", "stop").put("session", l.session)) }.start()
    }

    fun stopAllFor(deviceId: String) = live.value.values.filter { it.deviceId == deviceId }.forEach { stop(it.deviceId, it.plugin, notifyPeer = false) }

    fun stopAll() = live.value.values.forEach { stop(it.deviceId, it.plugin) }

    // --- the phone produces (after the user accepted) --------------------------

    /** Run on a background thread; [projection] only for the screen. */
    fun accept(ctx: Context, core: Core, r: StreamRequest, projection: MediaProjection? = null) {
        pending.update { l -> l.filterNot { it.key == r.key } }
        onRequestsChanged?.invoke()
        stop(r.deviceId, r.plugin, notifyPeer = false)
        val ch = try {
            core.callObject("streamListen", JSONObject().put("deviceId", r.deviceId))
        } catch (e: Exception) {
            Log.w(TAG, "streamListen failed", e)
            return
        }
        val channel = ch.optLong("channelId")
        val port = ch.optInt("port")
        // The producer ended by itself (computer gone, never connected, capture stopped).
        val ended = { Thread { stop(r.deviceId, r.plugin) }.start(); Unit }
        val (worker, out) = try {
            when (r.plugin) {
                "microphone" -> MicProducer(core, channel, ended) to
                    JSONObject().put("codec", "pcm_s16le").put("sampleRate", 48000).put("channels", 1)
                "camera" -> {
                    val w = r.params.optInt("width", 1280).coerceIn(320, 1920)
                    val h = r.params.optInt("height", 720).coerceIn(240, 1080)
                    val fps = r.params.optInt("fps", 30).coerceIn(10, 30)
                    val facing = r.params.optString("facing", "back")
                    CameraProducer(ctx, core, channel, w, h, fps, facing, ended) to
                        JSONObject().put("codec", "h264").put("width", w).put("height", h).put("fps", fps).put("facing", facing)
                }
                "phoneaudio" -> {
                    val p = projection ?: return
                    PlaybackProducer(core, channel, p, ownsProjection = !r.params.optBoolean("sharedProjection"), ended) to
                        JSONObject().put("codec", "pcm_s16le").put("sampleRate", 48000).put("channels", 2)
                }
                else -> {
                    val p = projection ?: return
                    val prod = ScreenProducer(ctx, core, channel, p, r.params.optInt("height", 1600).coerceIn(480, 1920), ended)
                    prod to JSONObject().put("codec", "h264").put("width", prod.width).put("height", prod.height).put("fps", 30)
                }
            }
        } catch (e: Exception) {
            Log.w(TAG, "cannot start ${r.plugin}", e)
            core.call("streamClose", JSONObject().put("channelId", channel))
            send(r.deviceId, r.plugin, JSONObject().put("action", "stop").put("session", r.session).put("reason", e.message ?: "errore"))
            return
        }
        synchronized(workers) { workers[r.key] = worker }
        live.update { it + (r.key to LiveStream(r.plugin, r.deviceId, r.session, outgoing = true)) }
        onChanged?.invoke()
        send(r.deviceId, r.plugin, JSONObject().put("action", "offer").put("session", r.session).put("port", port).put("params", out))
        // "Screen with audio": the phone's sound goes as its own stream, on the same capture consent.
        if (r.plugin == "phonescreen" && r.params.optBoolean("withAudio") && projection != null) {
            sharedAudio.add(r.deviceId)
            accept(ctx, core, newRequest(r.deviceId, r.deviceName, "phoneaudio", JSONObject().put("sharedProjection", true)), projection)
        }
    }

    // --- the phone consumes ------------------------------------------------------

    /** Ask a computer for its screen or audio; its offer arrives as a packet. */
    fun request(deviceId: String, plugin: String, params: JSONObject = JSONObject()) {
        Thread { send(deviceId, plugin, JSONObject().put("action", "request").put("session", newSession()).put("params", params)) }.start()
    }

    private fun consume(core: Core, deviceId: String, plugin: String, session: String, offer: JSONObject) {
        val key = "$plugin/$deviceId"
        stop(deviceId, plugin, notifyPeer = false)
        val channel = try {
            core.callObject("streamConnect", JSONObject().put("deviceId", deviceId).put("port", offer.optInt("port"))).optLong("channelId")
        } catch (e: Exception) {
            Log.w(TAG, "streamConnect failed", e)
            return
        }
        val params = offer.optJSONObject("params") ?: JSONObject()
        val worker: Worker = if (plugin == "desktop") {
            DesktopDecoder(core, channel, params.optInt("width", 1920), params.optInt("height", 1080)) { stop(deviceId, plugin, notifyPeer = false) }
                .also { desktopDecoders[key] = it }
        } else {
            AudioPlayer(core, channel, params.optInt("sampleRate", 48000), params.optInt("channels", 2)) { stop(deviceId, plugin, notifyPeer = false) }
        }
        synchronized(workers) { workers[key] = worker }
        live.update { it + (key to LiveStream(plugin, deviceId, session, outgoing = false)) }
    }

    // --- producers -----------------------------------------------------------------

    /** Tracks a producer's channel: false when it should stop writing. */
    private class Pace {
        private var pendingSince = 0L

        fun ok(result: Int): Boolean {
            if (result < 0) return false
            if (result == 1) {
                val now = System.currentTimeMillis()
                if (pendingSince == 0L) pendingSince = now
                return now - pendingSince < CONNECT_TIMEOUT_MS
            }
            pendingSince = Long.MAX_VALUE / 2 // connected once: drops are only slowness now
            return true
        }
    }

    private class MicProducer(private val core: Core, private val channel: Long, private val ended: () -> Unit) : Worker {
        @Volatile
        private var running = true
        private val pace = Pace()

        @SuppressLint("MissingPermission")
        private val rec = AudioRecord(
            MediaRecorder.AudioSource.VOICE_COMMUNICATION, 48000, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT,
            maxOf(AudioRecord.getMinBufferSize(48000, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT), 9600),
        )

        init {
            check(rec.state == AudioRecord.STATE_INITIALIZED) { "microfono non disponibile" }
            rec.startRecording()
            Thread({
                val buf = ByteArray(1920) // 20 ms
                while (running) {
                    val n = rec.read(buf, 0, buf.size)
                    if (n <= 0) continue
                    if (!pace.ok(core.streamWrite(channel, Frame.AUDIO, 0, System.nanoTime() / 1000, buf, n))) break
                }
                val byItself = running
                close()
                if (byItself) ended()
            }, "konnekta-mic").start()
        }

        private fun close() {
            running = false
            runCatching { rec.stop() }
            rec.release()
            runCatching { core.call("streamClose", JSONObject().put("channelId", channel)) }
        }

        override fun stop() {
            running = false
        }
    }

    /** H.264 encoder whose input is a Surface; output goes to the channel. */
    private open class SurfaceEncoder(
        private val core: Core, private val channel: Long, val width: Int, val height: Int, fps: Int, private val ended: () -> Unit,
    ) : Worker {
        @Volatile
        protected var running = true
        private val pace = Pace()
        private var config: ByteArray? = null
        private var connected = false
        private val codec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
        val input: Surface

        init {
            val f = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, width, height).apply {
                setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
                // ~0.15 bit per pixel: sharp text and motion without saturating Wi-Fi.
                setInteger(MediaFormat.KEY_BIT_RATE, (width.toLong() * height * fps * 15 / 100).toInt().coerceIn(3_000_000, 12_000_000))
                setInteger(MediaFormat.KEY_PRIORITY, 0) // real time
                setInteger(MediaFormat.KEY_FRAME_RATE, fps)
                setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 2)
                // Screen content: keep sending frames when nothing moves.
                setLong(MediaFormat.KEY_REPEAT_PREVIOUS_FRAME_AFTER, 100_000)
            }
            codec.configure(f, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            input = codec.createInputSurface()
            codec.start()
            Thread(::drain, "konnekta-enc").start()
        }

        private fun drain() {
            val info = MediaCodec.BufferInfo()
            var buf = ByteArray(256 * 1024)
            try {
                while (running) {
                    val i = codec.dequeueOutputBuffer(info, 50_000)
                    if (i < 0) continue
                    val out = codec.getOutputBuffer(i)
                    if (out != null && info.size > 0) {
                        if (buf.size < info.size) buf = ByteArray(info.size * 2)
                        out.position(info.offset)
                        out.get(buf, 0, info.size)
                        var flags = 0
                        val isConfig = info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
                        val isKey = info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
                        if (isConfig) flags = flags or Frame.CONFIG
                        if (isKey) flags = flags or Frame.KEYFRAME
                        if (isConfig) config = buf.copyOf(info.size)
                        // Android sends SPS/PPS once; repeat them before every keyframe so a
                        // computer that connects (or reconnects its decoder) later can start.
                        if (isKey && !isConfig) config?.let { core.streamWrite(channel, Frame.VIDEO, Frame.CONFIG or Frame.KEYFRAME, info.presentationTimeUs, it) }
                        val r = core.streamWrite(channel, Frame.VIDEO, flags, info.presentationTimeUs, buf, info.size)
                        if (!pace.ok(r)) break
                        if (r == 0 && !connected) {
                            connected = true
                            // First frame through: start the computer's decoder right away.
                            codec.setParameters(android.os.Bundle().apply { putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0) })
                        }
                    }
                    codec.releaseOutputBuffer(i, false)
                }
            } catch (e: Exception) {
                Log.w(TAG, "encoder stopped", e)
            }
            val byItself = running
            running = false
            onClosed()
            runCatching { codec.stop() }
            codec.release()
            input.release()
            runCatching { core.call("streamClose", JSONObject().put("channelId", channel)) }
            if (byItself) ended()
        }

        protected open fun onClosed() {}

        override fun stop() {
            running = false
        }
    }

    private class CameraProducer(ctx: Context, core: Core, channel: Long, w: Int, h: Int, private val fps: Int, facing: String, ended: () -> Unit) :
        SurfaceEncoder(core, channel, w, h, fps, ended) {
        private val thread = HandlerThread("konnekta-cam").apply { start() }
        private val handler = Handler(thread.looper)
        private var device: CameraDevice? = null
        /** The webcam page's preview, if open when the stream started. */
        private val preview = cameraPreview?.takeIf { it.isValid }

        init {
            val cm = ctx.getSystemService(CameraManager::class.java)
            val want = if (facing == "front") CameraCharacteristics.LENS_FACING_FRONT else CameraCharacteristics.LENS_FACING_BACK
            val id = cm.cameraIdList.firstOrNull { cm.getCameraCharacteristics(it).get(CameraCharacteristics.LENS_FACING) == want }
                ?: cm.cameraIdList.firstOrNull() ?: error("nessuna fotocamera")
            open(cm, id)
        }

        /*
         * The camera answers on its own thread, possibly after the stream
         * ended (the computer refused it, or the user stopped it at once): the
         * encoder's Surface is gone by then, so every step checks `running`
         * and never lets an exception escape (it would close the app).
         */
        @SuppressLint("MissingPermission")
        private fun open(cm: CameraManager, id: String) {
            cm.openCamera(id, object : CameraDevice.StateCallback() {
                override fun onOpened(d: CameraDevice) {
                    synchronized(this@CameraProducer) {
                        if (!running) {
                            runCatching { d.close() }
                            return
                        }
                        device = d
                    }
                    try {
                        @Suppress("DEPRECATION")
                        d.createCaptureSession(listOfNotNull(input, preview), object : CameraCaptureSession.StateCallback() {
                            override fun onConfigured(s: CameraCaptureSession) {
                                if (!running) return
                                runCatching {
                                    val req = d.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
                                        addTarget(input)
                                        preview?.let { p -> addTarget(p) }
                                        set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, Range(fps, fps))
                                    }.build()
                                    s.setRepeatingRequest(req, null, handler)
                                }.onFailure { Log.w(TAG, "camera request failed", it); stop() }
                            }

                            override fun onConfigureFailed(s: CameraCaptureSession) = stop()
                        }, handler)
                    } catch (e: Exception) {
                        Log.w(TAG, "camera session failed", e)
                        stop()
                    }
                }

                override fun onDisconnected(d: CameraDevice) = stop()
                override fun onError(d: CameraDevice, error: Int) = stop()
            }, handler)
        }

        override fun onClosed() {
            synchronized(this) { runCatching { device?.close() }; device = null }
            thread.quitSafely()
        }
    }

    private class ScreenProducer(ctx: Context, core: Core, channel: Long, private val projection: MediaProjection, maxSide: Int, ended: () -> Unit) :
        SurfaceEncoder(core, channel, size(ctx, maxSide).first, size(ctx, maxSide).second, 30, ended) {
        private var display: VirtualDisplay? = null

        companion object {
            /** Screen size scaled so the long side is at most [maxSide], even numbers. */
            fun size(ctx: Context, maxSide: Int): Pair<Int, Int> {
                val m = ctx.resources.displayMetrics
                val scale = minOf(1f, maxSide.toFloat() / maxOf(m.widthPixels, m.heightPixels))
                return ((m.widthPixels * scale).toInt() and 1.inv()) to ((m.heightPixels * scale).toInt() and 1.inv())
            }
        }

        init {
            projection.registerCallback(object : MediaProjection.Callback() {
                override fun onStop() = stop()
            }, Handler(android.os.Looper.getMainLooper()))
            display = projection.createVirtualDisplay(
                "konnekta", width, height, ctx.resources.displayMetrics.densityDpi,
                DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, input, null, null,
            )
        }

        override fun onClosed() {
            display?.release()
            runCatching { projection.stop() }
        }
    }

    /** What the phone is playing (Android 10+ playback capture), PCM s16le 48 kHz stereo. */
    private class PlaybackProducer(
        private val core: Core, private val channel: Long, private val projection: MediaProjection,
        private val ownsProjection: Boolean, private val ended: () -> Unit,
    ) : Worker {
        @Volatile
        private var running = true
        private val pace = Pace()

        @SuppressLint("MissingPermission")
        private val rec: AudioRecord = run {
            check(android.os.Build.VERSION.SDK_INT >= 29) { "serve Android 10 o successivo" }
            val cfg = android.media.AudioPlaybackCaptureConfiguration.Builder(projection)
                .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
                .addMatchingUsage(AudioAttributes.USAGE_GAME)
                .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
                .build()
            AudioRecord.Builder()
                .setAudioFormat(AudioFormat.Builder().setSampleRate(48000).setChannelMask(AudioFormat.CHANNEL_IN_STEREO)
                    .setEncoding(AudioFormat.ENCODING_PCM_16BIT).build())
                .setBufferSizeInBytes(19200)
                .setAudioPlaybackCaptureConfig(cfg)
                .build()
        }

        init {
            check(rec.state == AudioRecord.STATE_INITIALIZED) { "cattura dell'audio non disponibile" }
            if (ownsProjection) projection.registerCallback(object : MediaProjection.Callback() {
                override fun onStop() = stop()
            }, Handler(android.os.Looper.getMainLooper()))
            rec.startRecording()
            Thread({
                val buf = ByteArray(3840) // 20 ms stereo
                while (running) {
                    val n = rec.read(buf, 0, buf.size)
                    if (n < 0) break // the shared capture ended
                    if (n == 0) continue
                    if (!pace.ok(core.streamWrite(channel, Frame.AUDIO, 0, System.nanoTime() / 1000, buf, n))) break
                }
                val byItself = running
                running = false
                runCatching { rec.stop() }
                rec.release()
                if (ownsProjection) runCatching { projection.stop() }
                runCatching { core.call("streamClose", JSONObject().put("channelId", channel)) }
                if (byItself) ended()
            }, "konnekta-playback").start()
        }

        override fun stop() {
            running = false
        }
    }

    // --- consumers -----------------------------------------------------------------

    private class AudioPlayer(private val core: Core, private val channel: Long, rate: Int, channels: Int, private val ended: () -> Unit) : Worker {
        @Volatile
        private var running = true

        init {
            val mask = if (channels == 1) AudioFormat.CHANNEL_OUT_MONO else AudioFormat.CHANNEL_OUT_STEREO
            val track = AudioTrack.Builder()
                .setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build())
                .setAudioFormat(AudioFormat.Builder().setSampleRate(rate).setChannelMask(mask).setEncoding(AudioFormat.ENCODING_PCM_16BIT).build())
                .setBufferSizeInBytes(maxOf(AudioTrack.getMinBufferSize(rate, mask, AudioFormat.ENCODING_PCM_16BIT) * 2, 19200))
                .setTransferMode(AudioTrack.MODE_STREAM)
                .build()
            track.play()
            Thread({
                val buf = ByteArray(256 * 1024)
                val meta = ByteArray(2)
                val pts = LongArray(1)
                while (running) {
                    val n = core.streamRead(channel, buf, meta, pts, 500)
                    if (n < 0) break
                    if (n > 0 && meta[0].toInt() == Frame.AUDIO) track.write(buf, 0, n)
                }
                track.stop()
                track.release()
                runCatching { core.call("streamClose", JSONObject().put("channelId", channel)) }
                if (running) ended()
            }, "konnekta-audio").start()
        }

        override fun stop() {
            running = false
        }
    }

    /**
     * The computer's screen: one complete picture per frame (Annex B), and a
     * config frame with SPS + PPS before every keyframe. Each picture goes to
     * the decoder the moment it arrives, and a render thread shows each
     * decoded picture at once (Flux-style: nothing is held back for smooth
     * playback). Hardware decoder first; if it refuses the stream or shows
     * nothing, the software one takes over. Rebuilt when the Surface or the
     * stream's parameter sets change.
     */
    private class DesktopDecoder(private val core: Core, private val channel: Long, w: Int, h: Int, private val ended: () -> Unit) : Worker {
        @Volatile
        private var running = true
        @Volatile
        private var rebuild = true
        private var codec: MediaCodec? = null
        private var renderer: Thread? = null
        @Volatile
        private var rendering = false
        private var software = false
        private var sps: ByteArray? = null
        private var pps: ByteArray? = null
        private var size = w to h
        private var waitingKey = true
        private var queuedSinceBuild = 0
        private val rendered = java.util.concurrent.atomic.AtomicInteger()
        private var frames = 0L

        init {
            Thread(::run, "konnekta-desktop").start()
        }

        fun surfaceChanged() {
            rebuild = true
        }

        private fun release() {
            rendering = false
            renderer?.let { if (it !== Thread.currentThread()) runCatching { it.join(500) } }
            renderer = null
            codec?.let { runCatching { it.stop() }; it.release() }
            codec = null
        }

        private fun build(surface: Surface) {
            release()
            waitingKey = true
            queuedSinceBuild = 0
            rendered.set(0)
            val (fw, fh) = size
            val format = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, fw, fh).apply {
                sps?.let { setByteBuffer("csd-0", java.nio.ByteBuffer.wrap(it)) }
                pps?.let { setByteBuffer("csd-1", java.nio.ByteBuffer.wrap(it)) }
                setInteger(MediaFormat.KEY_PRIORITY, 0) // real time
                // Room for a detailed keyframe: a picture that does not fit is lost.
                setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, 4 * 1024 * 1024)
                if (android.os.Build.VERSION.SDK_INT >= 30) setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
            }
            val names = buildList {
                if (!software) android.media.MediaCodecList(android.media.MediaCodecList.REGULAR_CODECS)
                    .findDecoderForFormat(MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, fw, fh))?.let { add(it) }
                add("c2.android.avc.decoder")
                add("OMX.google.h264.decoder")
            }.distinct()
            for (n in names) {
                val c = runCatching { MediaCodec.createByCodecName(n) }.getOrNull() ?: continue
                try {
                    c.configure(format, surface, null, 0)
                    c.start()
                    codec = c
                    software = software || n.startsWith("c2.android") || n.startsWith("OMX.google")
                    rendering = true
                    renderer = Thread({ renderLoop(c) }, "konnekta-desktop-render").apply { isDaemon = true; start() }
                    Log.i(TAG, "remote desktop decoder $n ${fw}x$fh")
                    return
                } catch (e: Exception) {
                    Log.w(TAG, "decoder $n refused ${fw}x$fh", e)
                    c.release()
                }
            }
        }

        /** Shows every decoded picture as soon as it is ready. */
        private fun renderLoop(c: MediaCodec) {
            val info = MediaCodec.BufferInfo()
            try {
                while (rendering) {
                    val o = c.dequeueOutputBuffer(info, 10_000)
                    when {
                        o >= 0 -> {
                            c.releaseOutputBuffer(o, true)
                            rendered.incrementAndGet()
                        }
                        o == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> c.outputFormat.let { f ->
                            val cw = if (f.containsKey("crop-right")) f.getInteger("crop-right") - f.getInteger("crop-left") + 1 else f.getInteger(MediaFormat.KEY_WIDTH)
                            val ch = if (f.containsKey("crop-bottom")) f.getInteger("crop-bottom") - f.getInteger("crop-top") + 1 else f.getInteger(MediaFormat.KEY_HEIGHT)
                            desktopSize.value = cw to ch
                        }
                    }
                }
            } catch (e: Exception) {
                if (rendering) Log.i(TAG, "desktop render stopped: ${e.message}")
            }
        }

        /** Queue one picture; waits briefly for a free input buffer. */
        private fun feed(data: ByteArray, n: Int, key: Boolean) {
            val c = codec ?: return
            if (waitingKey && !key) return
            try {
                var i = -1
                while (running && !rebuild && i < 0) i = c.dequeueInputBuffer(10_000)
                if (i < 0) return
                val inBuf = c.getInputBuffer(i)!!
                inBuf.clear()
                if (n > inBuf.capacity()) {
                    c.queueInputBuffer(i, 0, 0, 0, 0)
                    waitingKey = true
                    return
                }
                inBuf.put(data, 0, n)
                // The time only orders the pictures: each shows when decoded.
                c.queueInputBuffer(i, 0, n, frames++ * 33_333, if (key) MediaCodec.BUFFER_FLAG_KEY_FRAME else 0)
                waitingKey = false
                queuedSinceBuild++
            } catch (e: Exception) {
                Log.w(TAG, "decoder failed, switching to software", e)
                software = true
                rebuild = true
            }
            // Hardware decoder that takes pictures but never shows one: use software.
            if (!software && queuedSinceBuild >= 30 && rendered.get() == 0) {
                Log.w(TAG, "hardware decoder shows nothing, switching to software")
                software = true
                rebuild = true
            }
        }

        /** SPS and PPS out of a config frame (Annex B, several NAL units). */
        private fun parameterSets(buf: ByteArray, n: Int) {
            var i = 0
            val starts = mutableListOf<Int>()
            while (i + 3 < n) {
                if (buf[i].toInt() == 0 && buf[i + 1].toInt() == 0 && buf[i + 2].toInt() == 1) {
                    starts += i
                    i += 3
                } else i++
            }
            starts.forEachIndexed { k, s ->
                // Include a 4-byte start code's leading zero.
                val from = if (s > 0 && buf[s - 1].toInt() == 0) s - 1 else s
                val to = (starts.getOrNull(k + 1)?.let { if (buf[it - 1].toInt() == 0) it - 1 else it }) ?: n
                val nal = buf.copyOfRange(from, to)
                val hdr = s - from + 3
                when (nal.getOrNull(hdr)?.toInt()?.and(0x1f)) {
                    7 -> if (!nal.contentEquals(sps)) {
                        sps = nal
                        H264.spsSize(nal, hdr)?.let { size = it }
                        rebuild = true
                    }
                    8 -> if (!nal.contentEquals(pps)) {
                        pps = nal
                        rebuild = true
                    }
                }
            }
        }

        private fun run() {
            val buf = ByteArray(8 * 1024 * 1024)
            val meta = ByteArray(2)
            val pts = LongArray(1)
            try {
                while (running) {
                    val n = core.streamRead(channel, buf, meta, pts, 100)
                    if (n < 0) break
                    if (rebuild && sps != null && pps != null) {
                        rebuild = false
                        desktopSurface?.takeIf { it.isValid }?.let(::build) ?: release()
                    }
                    if (n == 0 || meta[0].toInt() != Frame.VIDEO) continue
                    val flags = meta[1].toInt()
                    if (flags and Frame.CONFIG != 0) {
                        parameterSets(buf, n)
                        continue
                    }
                    feed(buf, n, flags and Frame.KEYFRAME != 0)
                }
            } catch (e: Exception) {
                Log.w(TAG, "remote desktop stopped", e)
            }
            release()
            runCatching { core.call("streamClose", JSONObject().put("channelId", channel)) }
            if (running) ended()
        }

        override fun stop() {
            running = false
        }
    }
}
