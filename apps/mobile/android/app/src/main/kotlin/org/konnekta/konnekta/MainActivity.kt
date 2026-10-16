package org.konnekta.konnekta

import android.app.NotificationManager
import android.content.BroadcastReceiver
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.database.ContentObserver
import android.media.AudioManager
import android.media.MediaScannerConnection
import android.media.RingtoneManager
import android.media.projection.MediaProjectionManager
import android.os.BatteryManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.provider.Settings
import android.telecom.TelecomManager
import android.telephony.TelephonyCallback
import android.telephony.TelephonyManager
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.EventChannel
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import io.flutter.view.TextureRegistry
import java.io.File
import java.security.MessageDigest

class MainActivity : FlutterActivity() {
    private val mic = MicCapture()
    private val camera by lazy { CameraStreamer(this) }
    private val screen by lazy { ScreenStreamer(this) }
    private var decoder: DesktopDecoder? = null
    private var decoderTexture: TextureRegistry.SurfaceTextureEntry? = null
    private var player: PcmPlayer? = null
    private var screenshotObserver: ContentObserver? = null
    private var pendingScreen: Triple<Int, Int, MethodChannel.Result>? = null
    private val sentScreenshots = HashSet<String>()

    override fun configureFlutterEngine(engine: FlutterEngine) {
        super.configureFlutterEngine(engine)
        EventChannel(engine.dartExecutor.binaryMessenger, "konnekta/native_events").setStreamHandler(object : EventChannel.StreamHandler {
            override fun onListen(args: Any?, sink: EventChannel.EventSink) { Bridge.sink = sink; onBridgeReady() }
            override fun onCancel(args: Any?) { Bridge.sink = null }
        })
        MethodChannel(engine.dartExecutor.binaryMessenger, "konnekta/native").setMethodCallHandler { call, result ->
            try { handle(call, result, engine) } catch (e: Exception) { result.error("native", e.message, null) }
        }
        val svc = Intent(this, KonnektaForegroundService::class.java)
        if (Build.VERSION.SDK_INT >= 26) startForegroundService(svc) else startService(svc)
        requestRuntimePermissions()
    }

    private fun requestRuntimePermissions() {
        val wanted = mutableListOf(
            android.Manifest.permission.READ_PHONE_STATE,
            android.Manifest.permission.READ_CONTACTS,
            android.Manifest.permission.ANSWER_PHONE_CALLS,
        )
        if (Build.VERSION.SDK_INT >= 33) {
            wanted += android.Manifest.permission.POST_NOTIFICATIONS
            wanted += android.Manifest.permission.READ_MEDIA_IMAGES
        } else {
            wanted += android.Manifest.permission.READ_EXTERNAL_STORAGE
        }
        val missing = wanted.filter { checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }
        if (missing.isNotEmpty()) requestPermissions(missing.toTypedArray(), 1)
    }

    private fun need(perm: String): Boolean {
        if (checkSelfPermission(perm) == PackageManager.PERMISSION_GRANTED) return true
        requestPermissions(arrayOf(perm), 2)
        return false
    }

    private fun onBridgeReady() {
        registerReceiver(object : BroadcastReceiver() {
            override fun onReceive(c: Context, i: Intent) = Bridge.emit(battery() + ("type" to "battery"))
        }, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        registerReceiver(object : BroadcastReceiver() {
            override fun onReceive(c: Context, i: Intent) {
                val nm = getSystemService(NotificationManager::class.java)
                Bridge.emit(mapOf("type" to "dnd", "enabled" to (nm.currentInterruptionFilter != NotificationManager.INTERRUPTION_FILTER_ALL)))
            }
        }, IntentFilter(NotificationManager.ACTION_INTERRUPTION_FILTER_CHANGED))
        watchCalls()
        setScreenshotWatch(true)
    }

    private fun battery(): Map<String, Any?> {
        val bm = getSystemService(BatteryManager::class.java)
        val level = bm.getIntProperty(BatteryManager.BATTERY_PROPERTY_CAPACITY)
        return mapOf("level" to level, "charging" to bm.isCharging, "low" to (level in 0..15))
    }

    // --- calls --------------------------------------------------------------

    private fun watchCalls() {
        if (Build.VERSION.SDK_INT < 31 || checkSelfPermission(android.Manifest.permission.READ_PHONE_STATE) != PackageManager.PERMISSION_GRANTED) return
        val tm = getSystemService(TelephonyManager::class.java)
        var last = TelephonyManager.CALL_STATE_IDLE
        tm.registerTelephonyCallback(mainExecutor, object : TelephonyCallback(), TelephonyCallback.CallStateListener {
            override fun onCallStateChanged(state: Int) {
                val body = when (state) {
                    TelephonyManager.CALL_STATE_RINGING -> mapOf("event" to "ringing")
                    TelephonyManager.CALL_STATE_OFFHOOK -> mapOf("event" to "talking")
                    else -> if (last == TelephonyManager.CALL_STATE_RINGING) mapOf("event" to "missedCall")
                            else mapOf("event" to "talking", "isCancel" to true)
                }
                last = state
                // Caller number is not exposed to TelephonyCallback on modern
                // Android; the notification mirror carries the caller name.
                Bridge.emit(mapOf("type" to "call", "body" to body))
            }
        })
    }

    // --- screenshots ----------------------------------------------------------

    private fun setScreenshotWatch(enabled: Boolean) {
        screenshotObserver?.let { contentResolver.unregisterContentObserver(it) }
        screenshotObserver = null
        if (!enabled) return
        val obs = object : ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) = checkLatestScreenshot()
        }
        contentResolver.registerContentObserver(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, true, obs)
        screenshotObserver = obs
    }

    private fun checkLatestScreenshot() {
        val proj = arrayOf(MediaStore.Images.Media.DATA, MediaStore.Images.Media.DATE_ADDED)
        contentResolver.query(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, proj, null, null,
            "${MediaStore.Images.Media.DATE_ADDED} DESC")?.use { c ->
            if (!c.moveToFirst()) return
            @Suppress("DEPRECATION")
            val path = c.getString(0) ?: return
            val added = c.getLong(1)
            if (!path.contains("Screenshot", ignoreCase = true)) return
            if (System.currentTimeMillis() / 1000 - added > 30 || !sentScreenshots.add(path)) return
            // The file may still be written: wait a little before hashing.
            Handler(Looper.getMainLooper()).postDelayed({
                Thread {
                    val f = File(path)
                    if (!f.exists()) return@Thread
                    val sha = MessageDigest.getInstance("SHA-256").digest(f.readBytes()).joinToString("") { "%02x".format(it) }
                    Bridge.emit(mapOf("type" to "screenshot", "path" to path, "sha256" to sha))
                }.start()
            }, 800)
        }
    }

    // --- method channel ---------------------------------------------------------

    private fun handle(call: MethodCall, result: MethodChannel.Result, engine: FlutterEngine) {
        fun <T> arg(k: String): T? = call.argument<T>(k)
        when (call.method) {
            "deviceName" -> result.success(Settings.Global.getString(contentResolver, Settings.Global.DEVICE_NAME) ?: Build.MODEL)
            "battery" -> result.success(battery())
            "resendNotifications" -> {
                val l = KonnektaNotificationListener.instance
                if (l == null) startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS))
                l?.resendAll(); result.success(l != null)
            }
            "dismissNotification" -> { KonnektaNotificationListener.instance?.dismiss(arg<String>("id") ?: ""); result.success(true) }
            "replyNotification" -> result.success(KonnektaNotificationListener.instance?.reply(arg("requestReplyId") ?: "", arg("message") ?: "") ?: false)
            "notificationAction" -> result.success(KonnektaNotificationListener.instance?.action(arg("key") ?: "", arg("action") ?: "") ?: false)
            "ring" -> {
                val am = getSystemService(AudioManager::class.java)
                am.setStreamVolume(AudioManager.STREAM_ALARM, am.getStreamMaxVolume(AudioManager.STREAM_ALARM), 0)
                val r = RingtoneManager.getRingtone(this, RingtoneManager.getDefaultUri(RingtoneManager.TYPE_ALARM))
                r.play()
                Handler(Looper.getMainLooper()).postDelayed({ r.stop() }, 30_000)
                result.success(true)
            }
            "muteRinger" -> {
                getSystemService(AudioManager::class.java).adjustStreamVolume(AudioManager.STREAM_RING, AudioManager.ADJUST_MUTE, 0)
                result.success(true)
            }
            "callAction" -> {
                val tm = getSystemService(TelecomManager::class.java)
                if (!need(android.Manifest.permission.ANSWER_PHONE_CALLS)) return result.success(false)
                @Suppress("DEPRECATION")
                when (arg<String>("action")) {
                    "accept" -> tm.acceptRingingCall()
                    "reject", "hangup" -> tm.endCall()
                    "mute" -> getSystemService(AudioManager::class.java).isMicrophoneMute = true
                    "unmute" -> getSystemService(AudioManager::class.java).isMicrophoneMute = false
                    "speaker" -> getSystemService(AudioManager::class.java).isSpeakerphoneOn = true
                }
                result.success(true)
            }
            "setDnd" -> {
                val nm = getSystemService(NotificationManager::class.java)
                if (!nm.isNotificationPolicyAccessGranted) {
                    startActivity(Intent(Settings.ACTION_NOTIFICATION_POLICY_ACCESS_SETTINGS))
                    return result.success(false)
                }
                nm.setInterruptionFilter(if (arg<Boolean>("enabled") == true) NotificationManager.INTERRUPTION_FILTER_PRIORITY else NotificationManager.INTERRUPTION_FILTER_ALL)
                result.success(true)
            }
            "setScreenshotWatch" -> { setScreenshotWatch(arg<Boolean>("enabled") == true); result.success(true) }
            "openUrl" -> result.success(true) // never auto-open remote links; Dart shows them instead
            "scanFile" -> { MediaScannerConnection.scanFile(this, arrayOf(arg<String>("path")), null, null); result.success(true) }
            "mediaControl" -> result.success(PhoneMedia.control(this, call.arguments as? Map<*, *> ?: emptyMap<String, Any>()))
            "startMic" -> {
                if (!need(android.Manifest.permission.RECORD_AUDIO)) return result.success(false)
                foreground(ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE)
                mic.start(); result.success(true)
            }
            "stopMic" -> { mic.stop(); result.success(true) }
            "startCamera" -> {
                if (!need(android.Manifest.permission.CAMERA)) return result.success(false)
                foreground(ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA)
                camera.start(arg("facing") ?: "back", arg("width") ?: 1280, arg("height") ?: 720, arg("fps") ?: 30)
                result.success(true)
            }
            "stopCamera" -> { camera.stop(); result.success(true) }
            "startScreenCapture" -> {
                val mpm = getSystemService(MediaProjectionManager::class.java)
                pendingScreen = Triple(arg("width") ?: 1280, arg("height") ?: 720, result)
                @Suppress("DEPRECATION")
                startActivityForResult(mpm.createScreenCaptureIntent(), 42)
            }
            "stopScreenCapture" -> { screen.stop(); result.success(true) }
            "startDecoder" -> {
                decoder?.stop(); decoderTexture?.release()
                val entry = engine.renderer.createSurfaceTexture()
                decoderTexture = entry
                decoder = DesktopDecoder(entry.surfaceTexture(), arg("width") ?: 0, arg("height") ?: 0)
                result.success(entry.id())
            }
            "decodeFrame" -> {
                decoder?.feed(arg<ByteArray>("data") ?: ByteArray(0), (arg<Number>("pts") ?: 0).toLong(), arg<Int>("flags") ?: 0)
                result.success(null)
            }
            "stopDecoder" -> { decoder?.stop(); decoder = null; decoderTexture?.release(); decoderTexture = null; result.success(true) }
            "startAudioPlayback" -> { player?.stop(); player = PcmPlayer(arg("sampleRate") ?: 48000, arg("channels") ?: 2); result.success(true) }
            "playPcm" -> { player?.write(arg<ByteArray>("data") ?: ByteArray(0)); result.success(null) }
            "stopAudioPlayback" -> { player?.stop(); player = null; result.success(true) }
            else -> result.notImplemented()
        }
    }

    private fun foreground(type: Int) {
        val i = Intent(this, KonnektaForegroundService::class.java)
            .putExtra(KonnektaForegroundService.EXTRA_TYPES, type or ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        if (Build.VERSION.SDK_INT >= 26) startForegroundService(i) else startService(i)
    }

    @Deprecated("Deprecated in Java")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != 42) return
        val (w, h, result) = pendingScreen ?: return
        pendingScreen = null
        if (resultCode != RESULT_OK || data == null) return result.success(false)
        foreground(ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION)
        // The projection must be created after the FGS is running (Android 14).
        Handler(Looper.getMainLooper()).postDelayed({
            val p = getSystemService(MediaProjectionManager::class.java).getMediaProjection(resultCode, data)
            screen.start(p, w, h, 30)
            result.success(true)
        }, 300)
    }

    override fun onDestroy() {
        mic.stop(); camera.stop(); screen.stop(); decoder?.stop(); player?.stop()
        super.onDestroy()
    }
}

/** Lets the computer control media playing on the phone. */
object PhoneMedia {
    fun control(ctx: Context, body: Map<*, *>): Boolean {
        val msm = ctx.getSystemService(android.media.session.MediaSessionManager::class.java)
        val sessions = try {
            msm.getActiveSessions(ComponentName(ctx, KonnektaNotificationListener::class.java))
        } catch (_: SecurityException) { return false }
        val player = body["player"] as? String
        val s = sessions.firstOrNull { player == null || it.packageName == player } ?: sessions.firstOrNull() ?: return false
        val tc = s.transportControls
        when (body["action"] as? String) {
            "Play" -> tc.play(); "Pause" -> tc.pause(); "Stop" -> tc.stop()
            "Next" -> tc.skipToNext(); "Previous" -> tc.skipToPrevious()
            "PlayPause" -> if (s.playbackState?.state == android.media.session.PlaybackState.STATE_PLAYING) tc.pause() else tc.play()
        }
        (body["setVolume"] as? Number)?.let {
            val am = ctx.getSystemService(AudioManager::class.java)
            val max = am.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
            am.setStreamVolume(AudioManager.STREAM_MUSIC, it.toInt() * max / 100, 0)
        }
        return true
    }
}
