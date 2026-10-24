package org.konnekta.app.service

import android.annotation.SuppressLint
import android.content.Context
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.os.Handler
import android.os.HandlerThread
import android.os.PowerManager
import android.util.Log
import android.util.Range
import android.util.Size
import android.view.Surface
import org.json.JSONObject
import org.konnekta.app.core.Core
import org.konnekta.app.core.Frame

/** Lens, picture size and frame rate of the webcam. */
data class CameraConfig(val front: Boolean, val width: Int, val height: Int, val fps: Int) {
    companion object {
        fun from(p: JSONObject) = CameraConfig(
            p.optString("facing", "back") == "front",
            p.optInt("width", 1280).coerceIn(320, 1920),
            p.optInt("height", 720).coerceIn(240, 1080),
            p.optInt("fps", 30).coerceIn(10, 30),
        )
    }
}

/** What the camera can do: the size closest to the wish and a frame-rate range it accepts. */
object CameraCaps {
    fun cameraId(cm: CameraManager, front: Boolean): String? {
        val want = if (front) CameraCharacteristics.LENS_FACING_FRONT else CameraCharacteristics.LENS_FACING_BACK
        return cm.cameraIdList.firstOrNull { cm.getCameraCharacteristics(it).get(CameraCharacteristics.LENS_FACING) == want }
            ?: cm.cameraIdList.firstOrNull()
    }

    /** Supported output size for [target] closest to w x h (same shape first). */
    fun size(cm: CameraManager, id: String, w: Int, h: Int, target: Class<*>): Size {
        val map = cm.getCameraCharacteristics(id).get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
        val sizes = map?.getOutputSizes(target)?.toList().orEmpty().filter { it.width <= 1920 && it.height <= 1080 }
        if (sizes.isEmpty()) return Size(w, h)
        val ratio = w.toFloat() / h
        return sizes.minBy { s ->
            val shape = kotlin.math.abs(s.width.toFloat() / s.height - ratio) * 4000
            shape + kotlin.math.abs(s.width * s.height - w * h) / 1000f
        }
    }

    /**
     * An auto-exposure range the camera lists: the steadiest one reaching
     * [fps] (asking for a range it does not list stops the camera).
     */
    fun fpsRange(cm: CameraManager, id: String, fps: Int): Range<Int>? {
        val ranges = cm.getCameraCharacteristics(id).get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES)
            ?.toList().orEmpty()
        if (ranges.isEmpty()) return null
        return ranges.filter { it.upper == fps }.maxByOrNull { it.lower }
            ?: ranges.filter { fps in it.lower..it.upper }.minByOrNull { it.upper - it.lower }
            ?: ranges.minBy { kotlin.math.abs(it.upper - fps) }
    }
}

/**
 * The phone camera as the computer's webcam. Lens, size and frame rate
 * change while it runs (the computer's camera just carries on); the
 * preview of the webcam page comes and goes without touching the stream,
 * and a wake lock keeps it going with the app in the background or the
 * screen off (the service holds the camera foreground type).
 *
 * Everything about the camera happens on one thread. A camera that answers
 * after we no longer want it is closed there, so it is never left open.
 */
class CameraStream(
    private val ctx: Context, private val core: Core, private val channel: Long,
    @Volatile var config: CameraConfig, private val ended: () -> Unit,
) : Streams.Worker {
    private val thread = HandlerThread("konnekta-cam").apply { start() }
    private val handler = Handler(thread.looper)
    private val cm = ctx.getSystemService(CameraManager::class.java)
    private val wake = ctx.getSystemService(PowerManager::class.java)
        .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "konnekta:webcam").apply { setReferenceCounted(false) }
    /** Wi-Fi stays fast with the screen off. */
    @Suppress("DEPRECATION")
    private val wifi = ctx.applicationContext.getSystemService(android.net.wifi.WifiManager::class.java)?.createWifiLock(
        if (android.os.Build.VERSION.SDK_INT >= 29) android.net.wifi.WifiManager.WIFI_MODE_FULL_LOW_LATENCY
        else android.net.wifi.WifiManager.WIFI_MODE_FULL_HIGH_PERF, "konnekta:webcam",
    )?.apply { setReferenceCounted(false) }

    @Volatile
    private var running = true
    private val pace = Pace()
    private var encoder: Encoder? = null
    private var device: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    private var preview: Surface? = Streams.cameraPreview?.takeIf { it.isValid }
    /** Bumped at every (re)open: answers for an older one are thrown away. */
    private var generation = 0
    private var opening = false
    private var quitWhenOpened = false

    /** Size actually sent (the camera's closest to the wish). */
    @Volatile
    var size: Size = Size(config.width, config.height)
        private set

    init {
        // Known at once: the offer to the computer carries it.
        CameraCaps.cameraId(cm, config.front)?.let { size = CameraCaps.size(cm, it, config.width, config.height, MediaCodec::class.java) }
        wake.acquire(12 * 3600 * 1000L)
        runCatching { wifi?.acquire() }
        handler.post {
            startEncoder()
            openCamera()
        }
    }

    // --- encoder -------------------------------------------------------------------

    private inner class Encoder(val width: Int, val height: Int, fps: Int) {
        val codec: MediaCodec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
        val input: Surface
        @Volatile
        var draining = true
        private var config: ByteArray? = null
        private val drain: Thread

        init {
            val f = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, width, height).apply {
                setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
                setInteger(MediaFormat.KEY_BIT_RATE, (width.toLong() * height * fps * 15 / 100).toInt().coerceIn(2_000_000, 10_000_000))
                setInteger(MediaFormat.KEY_PRIORITY, 0)
                setInteger(MediaFormat.KEY_FRAME_RATE, fps)
                setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 1)
                setLong(MediaFormat.KEY_REPEAT_PREVIOUS_FRAME_AFTER, 100_000)
            }
            codec.configure(f, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            input = codec.createInputSurface()
            codec.start()
            drain = Thread(::drainLoop, "konnekta-cam-enc").apply { start() }
        }

        private fun drainLoop() {
            val info = MediaCodec.BufferInfo()
            var buf = ByteArray(256 * 1024)
            try {
                while (draining) {
                    val i = codec.dequeueOutputBuffer(info, 50_000)
                    if (i < 0) continue
                    val out = codec.getOutputBuffer(i)
                    if (out != null && info.size > 0) {
                        if (buf.size < info.size) buf = ByteArray(info.size * 2)
                        out.position(info.offset)
                        out.get(buf, 0, info.size)
                        val isConfig = info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
                        val isKey = info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
                        var flags = 0
                        if (isConfig) flags = flags or Frame.CONFIG
                        if (isKey) flags = flags or Frame.KEYFRAME
                        if (isConfig) config = buf.copyOf(info.size)
                        // SPS/PPS before every keyframe: the computer's decoder
                        // (re)starts there, also after a change of size.
                        if (isKey && !isConfig) config?.let { core.streamWrite(channel, Frame.VIDEO, Frame.CONFIG or Frame.KEYFRAME, info.presentationTimeUs, it) }
                        val r = core.streamWrite(channel, Frame.VIDEO, flags, info.presentationTimeUs, buf, info.size)
                        if (!pace.ok(r)) {
                            codec.releaseOutputBuffer(i, false)
                            if (running) {
                                running = false
                                handler.post { shutdown(byItself = true) }
                            }
                            break
                        }
                    }
                    codec.releaseOutputBuffer(i, false)
                }
            } catch (e: Exception) {
                if (draining) Log.w(Streams.TAG, "camera encoder stopped", e)
            }
        }

        fun release() {
            draining = false
            runCatching { drain.join(500) }
            runCatching { codec.stop() }
            runCatching { codec.release() }
            input.release()
        }
    }

    /** Same rule as the other producers: never connected in time, or the channel is gone. */
    private class Pace {
        private var pendingSince = 0L
        fun ok(result: Int): Boolean {
            if (result < 0) return false
            if (result == 1) {
                val now = System.currentTimeMillis()
                if (pendingSince == 0L) pendingSince = now
                return now - pendingSince < 15_000
            }
            pendingSince = Long.MAX_VALUE / 2
            return true
        }
    }

    private fun startEncoder() {
        val id = CameraCaps.cameraId(cm, config.front) ?: return fail("nessuna fotocamera")
        val s = CameraCaps.size(cm, id, config.width, config.height, MediaCodec::class.java)
        size = s
        encoder = try {
            Encoder(s.width, s.height, config.fps)
        } catch (e: Exception) {
            return fail("encoder video non disponibile: ${e.message}")
        }
    }

    // --- camera --------------------------------------------------------------------

    @SuppressLint("MissingPermission")
    private fun openCamera() {
        if (!running) return
        val id = CameraCaps.cameraId(cm, config.front) ?: return fail("nessuna fotocamera")
        val gen = ++generation
        opening = true
        try {
            cm.openCamera(id, object : CameraDevice.StateCallback() {
                override fun onOpened(d: CameraDevice) {
                    opening = false
                    if (gen != generation || !running) {
                        runCatching { d.close() }
                        if (quitWhenOpened) thread.quitSafely()
                        return
                    }
                    device = d
                    configureSession(id)
                }

                override fun onDisconnected(d: CameraDevice) {
                    opening = false
                    runCatching { d.close() }
                    if (gen == generation && running) {
                        // Another app took the camera: try again shortly.
                        device = null
                        handler.postDelayed({ if (gen == generation) openCamera() }, 1500)
                    } else if (quitWhenOpened) thread.quitSafely()
                }

                override fun onError(d: CameraDevice, error: Int) {
                    opening = false
                    runCatching { d.close() }
                    if (gen == generation && running) {
                        Log.w(Streams.TAG, "camera error $error")
                        device = null
                        handler.postDelayed({ if (gen == generation) openCamera() }, 1500)
                    } else if (quitWhenOpened) thread.quitSafely()
                }
            }, handler)
        } catch (e: Exception) {
            opening = false
            fail("fotocamera non disponibile: ${e.message}")
        }
    }

    private fun configureSession(id: String) {
        val d = device ?: return
        val enc = encoder ?: return
        runCatching { session?.close() }
        session = null
        val gen = generation
        val targets = listOfNotNull(enc.input, preview?.takeIf { it.isValid })
        try {
            @Suppress("DEPRECATION")
            d.createCaptureSession(targets, object : CameraCaptureSession.StateCallback() {
                override fun onConfigured(s: CameraCaptureSession) {
                    if (gen != generation || !running) {
                        runCatching { s.close() }
                        return
                    }
                    session = s
                    try {
                        val req = d.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
                            targets.forEach { addTarget(it) }
                            CameraCaps.fpsRange(cm, id, config.fps)?.let { set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, it) }
                            set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO)
                        }.build()
                        s.setRepeatingRequest(req, null, handler)
                    } catch (e: Exception) {
                        Log.w(Streams.TAG, "camera request failed", e)
                        // A preview that went away: carry on without it.
                        if (preview != null) {
                            preview = null
                            configureSession(id)
                        }
                    }
                }

                override fun onConfigureFailed(s: CameraCaptureSession) {
                    Log.w(Streams.TAG, "camera session refused (${targets.size} targets)")
                    if (gen == generation && preview != null) {
                        preview = null
                        configureSession(id)
                    }
                }
            }, handler)
        } catch (e: Exception) {
            Log.w(Streams.TAG, "camera session failed", e)
            if (preview != null) {
                preview = null
                configureSession(id)
            }
        }
    }

    private fun closeCamera() {
        generation++
        runCatching { session?.close() }
        session = null
        runCatching { device?.close() }
        device = null
    }

    // --- control from the UI -----------------------------------------------------------

    /** New lens, size or frame rate while streaming. */
    fun reconfigure(c: CameraConfig) = handler.post {
        if (!running || c == config) return@post
        val newSize = c.width != config.width || c.height != config.height || c.fps != config.fps
        closeCamera()
        config = c
        if (newSize) {
            encoder?.release()
            encoder = null
            startEncoder()
        }
        openCamera()
    }

    /** The webcam page opened or closed its preview. */
    fun previewChanged(s: Surface?) = handler.post {
        if (!running) return@post
        preview = s?.takeIf { it.isValid }
        val d = device ?: return@post
        configureSession(d.id)
    }

    private fun fail(why: String) {
        Log.w(Streams.TAG, "webcam: $why")
        if (running) {
            running = false
            shutdown(byItself = true)
        }
    }

    private fun shutdown(byItself: Boolean) {
        closeCamera()
        encoder?.release()
        encoder = null
        runCatching { core.call("streamClose", JSONObject().put("channelId", channel)) }
        runCatching { if (wake.isHeld) wake.release() }
        runCatching { if (wifi?.isHeld == true) wifi.release() }
        // A camera still opening answers later on this thread: close it then.
        if (opening) quitWhenOpened = true else thread.quitSafely()
        if (byItself) ended()
    }

    override fun stop() {
        if (!running) return
        running = false
        handler.post { shutdown(byItself = false) }
    }
}
