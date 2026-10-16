package org.konnekta.konnekta

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.SurfaceTexture
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
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.view.Surface
import java.nio.ByteBuffer

const val FRAME_AUDIO = 1
const val FRAME_VIDEO = 2
const val FLAG_KEY = 1
const val FLAG_CONFIG = 2

/** Phone microphone -> PCM s16le 48 kHz mono, 10 ms chunks. */
class MicCapture {
    @Volatile private var running = false
    private var thread: Thread? = null

    @SuppressLint("MissingPermission")
    fun start() {
        if (running) return
        val rate = 48000
        val chunk = rate / 100 * 2
        val min = AudioRecord.getMinBufferSize(rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        val rec = AudioRecord(MediaRecorder.AudioSource.VOICE_COMMUNICATION, rate, AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_16BIT, maxOf(min, chunk * 4))
        running = true
        thread = Thread {
            val buf = ByteArray(chunk)
            rec.startRecording()
            while (running) {
                var off = 0
                while (off < chunk && running) {
                    val n = rec.read(buf, off, chunk - off)
                    if (n <= 0) break
                    off += n
                }
                if (off == chunk) Bridge.frame("microphone", FRAME_AUDIO, 0, System.nanoTime() / 1000, buf.copyOf())
            }
            rec.stop()
            rec.release()
        }.apply { name = "konnekta-mic"; priority = Thread.MAX_PRIORITY; start() }
    }

    fun stop() { running = false; thread = null }
}

/** Shared H.264 encoder with a Surface input; NAL units go to Dart. */
class H264Encoder(private val plugin: String, width: Int, height: Int, fps: Int, bitrate: Int) {
    val codec: MediaCodec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
    val surface: Surface
    @Volatile private var running = true

    init {
        val f = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, width, height)
        f.setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
        f.setInteger(MediaFormat.KEY_BIT_RATE, bitrate)
        f.setInteger(MediaFormat.KEY_FRAME_RATE, fps)
        f.setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 2)
        f.setInteger(MediaFormat.KEY_LATENCY, 0)
        f.setInteger(MediaFormat.KEY_PRIORITY, 0)
        codec.configure(f, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        surface = codec.createInputSurface()
        codec.start()
        Thread {
            val info = MediaCodec.BufferInfo()
            while (running) {
                val i = try { codec.dequeueOutputBuffer(info, 10_000) } catch (_: IllegalStateException) { break }
                if (i < 0) continue
                val out: ByteBuffer = codec.getOutputBuffer(i) ?: continue
                val data = ByteArray(info.size)
                out.position(info.offset); out.get(data)
                var flags = 0
                if (info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0) flags = flags or FLAG_KEY
                if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) flags = flags or FLAG_CONFIG or FLAG_KEY
                Bridge.frame(plugin, FRAME_VIDEO, flags, info.presentationTimeUs, data)
                codec.releaseOutputBuffer(i, false)
            }
        }.apply { name = "konnekta-enc-$plugin"; start() }
    }

    fun requestKeyframe() = codec.setParameters(Bundle().apply { putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0) })

    fun stop() {
        running = false
        try { codec.stop() } catch (_: Exception) {}
        codec.release()
        surface.release()
    }
}

/** Phone camera as wireless webcam (Camera2 -> encoder surface). */
class CameraStreamer(private val ctx: Context) {
    private var device: CameraDevice? = null
    private var encoder: H264Encoder? = null
    private val thread = HandlerThread("konnekta-camera").apply { start() }
    private val handler = Handler(thread.looper)

    @SuppressLint("MissingPermission")
    fun start(facing: String, width: Int, height: Int, fps: Int) {
        stop()
        val cm = ctx.getSystemService(CameraManager::class.java)
        val want = if (facing == "front") CameraCharacteristics.LENS_FACING_FRONT else CameraCharacteristics.LENS_FACING_BACK
        val id = cm.cameraIdList.firstOrNull { cm.getCameraCharacteristics(it).get(CameraCharacteristics.LENS_FACING) == want }
            ?: cm.cameraIdList.first()
        val enc = H264Encoder("camera", width, height, fps, width * height * 3)
        encoder = enc
        cm.openCamera(id, object : CameraDevice.StateCallback() {
            override fun onOpened(cam: CameraDevice) {
                device = cam
                @Suppress("DEPRECATION")
                cam.createCaptureSession(listOf(enc.surface), object : CameraCaptureSession.StateCallback() {
                    override fun onConfigured(s: CameraCaptureSession) {
                        val req = cam.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
                            addTarget(enc.surface)
                            set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, android.util.Range(fps, fps))
                        }
                        s.setRepeatingRequest(req.build(), null, handler)
                    }
                    override fun onConfigureFailed(s: CameraCaptureSession) = Bridge.error("Configurazione fotocamera non riuscita")
                }, handler)
            }
            override fun onDisconnected(cam: CameraDevice) = cam.close()
            override fun onError(cam: CameraDevice, error: Int) { cam.close(); Bridge.error("Errore fotocamera $error") }
        }, handler)
    }

    fun stop() {
        device?.close(); device = null
        encoder?.stop(); encoder = null
    }
}

/** Phone screen -> encoder (MediaProjection granted by the user). */
class ScreenStreamer(private val ctx: Context) {
    private var display: VirtualDisplay? = null
    private var encoder: H264Encoder? = null
    private var projection: MediaProjection? = null

    fun start(p: MediaProjection, width: Int, height: Int, fps: Int) {
        stop()
        projection = p
        val metrics = ctx.resources.displayMetrics
        val scale = minOf(width.toFloat() / metrics.widthPixels, height.toFloat() / metrics.heightPixels, 1f)
        val w = ((metrics.widthPixels * scale).toInt() / 16) * 16
        val h = ((metrics.heightPixels * scale).toInt() / 16) * 16
        val enc = H264Encoder("phonescreen", w, h, fps, w * h * 2)
        encoder = enc
        p.registerCallback(object : MediaProjection.Callback() { override fun onStop() = stop() }, null)
        display = p.createVirtualDisplay("konnekta", w, h, metrics.densityDpi,
            DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, enc.surface, null, null)
    }

    fun stop() {
        display?.release(); display = null
        encoder?.stop(); encoder = null
        projection?.stop(); projection = null
    }
}

/** Decodes the computer's desktop stream into a Flutter texture. */
class DesktopDecoder(texture: SurfaceTexture, width: Int, height: Int) {
    private val surface = Surface(texture)
    private val codec = MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
    private var configured = false
    private val w = if (width > 0) width else 1920
    private val h = if (height > 0) height else 1080

    init { texture.setDefaultBufferSize(w, h) }

    @Synchronized
    fun feed(data: ByteArray, ptsUs: Long, flags: Int) {
        if (!configured) {
            // Wait for SPS/PPS before starting the decoder.
            if (flags and FLAG_CONFIG == 0) return
            val f = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, w, h)
            f.setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
            codec.configure(f, surface, null, 0)
            codec.start()
            configured = true
        }
        val i = codec.dequeueInputBuffer(5_000)
        if (i >= 0) {
            val buf = codec.getInputBuffer(i)!!
            buf.clear(); buf.put(data)
            codec.queueInputBuffer(i, 0, data.size, ptsUs, 0)
        }
        val info = MediaCodec.BufferInfo()
        var o = codec.dequeueOutputBuffer(info, 0)
        while (o >= 0) {
            codec.releaseOutputBuffer(o, true)
            o = codec.dequeueOutputBuffer(info, 0)
        }
    }

    @Synchronized
    fun stop() {
        try { if (configured) codec.stop() } catch (_: Exception) {}
        codec.release()
        surface.release()
    }
}

/** Plays the computer's audio (PCM s16le 48 kHz). */
class PcmPlayer(rate: Int, channels: Int) {
    private val track: AudioTrack = AudioTrack.Builder()
        .setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build())
        .setAudioFormat(AudioFormat.Builder().setSampleRate(rate).setEncoding(AudioFormat.ENCODING_PCM_16BIT)
            .setChannelMask(if (channels == 2) AudioFormat.CHANNEL_OUT_STEREO else AudioFormat.CHANNEL_OUT_MONO).build())
        .setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY)
        .setTransferMode(AudioTrack.MODE_STREAM)
        .build()

    init { track.play() }

    fun write(pcm: ByteArray) { track.write(pcm, 0, pcm.size, AudioTrack.WRITE_NON_BLOCKING) }

    fun stop() { track.stop(); track.release() }
}
