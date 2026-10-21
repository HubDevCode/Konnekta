package org.konnekta.app.service

import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ServiceInfo
import android.media.AudioAttributes
import android.media.Ringtone
import android.media.RingtoneManager
import android.net.Uri
import android.net.wifi.WifiManager
import android.os.BatteryManager
import android.os.Build
import android.os.Environment
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import kotlinx.coroutines.flow.MutableStateFlow
import org.json.JSONArray
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.R
import org.konnekta.app.approve.ApproveActivity
import org.konnekta.app.approve.ApproveKeys
import org.konnekta.app.approve.PendingApprovals
import org.konnekta.app.core.Core
import org.konnekta.app.core.Types
import org.konnekta.app.state.Store
import org.konnekta.app.ui.MainActivity
import java.io.File
import java.util.concurrent.Executors

/**
 * Keeps the core running while the app is in the background, and handles
 * what the computer sends to the phone. The core does the networking,
 * pairing and permission checks; here are only the Android integrations.
 */
class KonnektaService : Service() {
    companion object {
        private const val TAG = "Konnekta"
        private const val N_SERVICE = 1
        private const val N_PING = 2
        private const val N_RING = 3
        private const val N_APPROVE = 4
        private const val N_PAIR = 5
        private const val N_SHARE = 6
        const val N_PHOTO = 7
        const val ACTION_STOP_RING = "org.konnekta.app.STOP_RING"

        /** Pairing request waiting for the user: {deviceId, deviceName, verificationKey, confirm}. */
        val pairPrompt = MutableStateFlow<JSONObject?>(null)

        fun start(context: Context) {
            context.startForegroundService(Intent(context, KonnektaService::class.java))
        }

        const val ACTION_REFRESH = "org.konnekta.app.REFRESH"
        private const val ACTION_ACCEPT_STREAM = "org.konnekta.app.ACCEPT_STREAM"
        const val N_STREAM = 8
        private const val ACTION_DECLINE_STREAM = "org.konnekta.app.DECLINE_STREAM"
        private const val ACTION_DECLINE_PHOTO = "org.konnekta.app.DECLINE_PHOTO"
        private const val ACTION_PAIR_ANSWER = "org.konnekta.app.PAIR_ANSWER"

        /**
         * The user accepted a stream request (the app is on screen, so the
         * service may add the microphone / camera / screen capture types).
         */
        fun acceptStream(context: Context, r: StreamRequest, projection: Intent? = null, resultCode: Int = 0) {
            context.startForegroundService(
                Intent(context, KonnektaService::class.java).setAction(ACTION_ACCEPT_STREAM)
                    .putExtra("device", r.deviceId).putExtra("name", r.deviceName).putExtra("plugin", r.plugin)
                    .putExtra("session", r.session).putExtra("params", r.params.toString())
                    .putExtra("projection", projection).putExtra("resultCode", resultCode),
            )
        }

        /** Phone permissions changed: (re)start the features that need them. */
        fun refreshFeatures(context: Context) {
            context.startService(Intent(context, KonnektaService::class.java).setAction(ACTION_REFRESH))
        }

        /** Start the core again (a new device name is announced only when links open). */
        fun restart(context: Context) {
            context.stopService(Intent(context, KonnektaService::class.java))
            Handler(Looper.getMainLooper()).postDelayed({ start(context) }, 1500)
        }

        fun stopRinging(context: Context) {
            context.startService(Intent(context, KonnektaService::class.java).setAction(ACTION_STOP_RING))
        }
    }

    @Volatile
    private var running = true
    private val io = Executors.newSingleThreadExecutor()
    private val transfers = Executors.newFixedThreadPool(2)
    private val main = Handler(Looper.getMainLooper())
    private var multicast: WifiManager.MulticastLock? = null
    private var ringtone: Ringtone? = null
    private var lastBattery: JSONObject? = null
    private val nm by lazy { getSystemService(NotificationManager::class.java) }
    private var media: PhoneMedia? = null
    private var calls: Calls? = null
    private var sms: Sms? = null
    private var dnd: Dnd? = null
    private var shots: Screenshots? = null

    /** Android integrations; each one checks its own permission. */
    private fun restartFeatures() {
        media?.stop(); calls?.stop(); sms?.stop(); dnd?.stop(); shots?.stop()
        media = PhoneMedia(this).also { it.start() }
        calls = Calls(this).also { it.start() }
        sms = Sms(this).also { it.start() }
        dnd = Dnd(this).also { it.start() }
        shots = Screenshots(this).also { it.start() }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        val n = NotificationCompat.Builder(this, KonnektaApp.CH_SERVICE)
            .setSmallIcon(R.drawable.ic_stat)
            .setContentTitle("Konnekta")
            .setContentText("Pronto a collegarsi ai tuoi computer")
            .setContentIntent(openApp())
            .setOngoing(true)
            .build()
        ServiceCompat.startForeground(this, N_SERVICE, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
        // UDP announcements and mDNS need multicast on Wi-Fi.
        multicast = getSystemService(WifiManager::class.java)?.createMulticastLock("konnekta")?.apply {
            setReferenceCounted(false)
            acquire()
        }
        Thread(::run, "konnekta-core").start()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP_RING) stopRing()
        if (intent?.action == ACTION_REFRESH) restartFeatures()
        if (intent?.action == ACTION_ACCEPT_STREAM) onAcceptStream(intent)
        if (intent?.action == ACTION_DECLINE_STREAM) {
            Streams.pending.value.firstOrNull { it.deviceId == intent.getStringExtra("device") && it.plugin == intent.getStringExtra("plugin") }
                ?.let(Streams::decline)
        }
        if (intent?.action == ACTION_DECLINE_PHOTO) {
            val dev = intent.getStringExtra("device").orEmpty()
            Store.dismissKind("photo")
            nm.cancel(N_PHOTO)
            transfers.execute { runCatching { KonnektaApp.core?.sendPacket(dev, Types.PHOTO, JSONObject().put("cancelled", true)) } }
        }
        if (intent?.action == ACTION_PAIR_ANSWER) {
            val dev = intent.getStringExtra("device").orEmpty()
            val key = intent.getStringExtra("key").orEmpty()
            val method = if (intent.getBooleanExtra("accept", false)) "acceptPair" else "rejectPair"
            pairPrompt.value = null
            nm.cancel(N_PAIR)
            transfers.execute { runCatching { KonnektaApp.core?.call(method, JSONObject().put("deviceId", dev).put("key", key)) } }
        }
        return START_STICKY
    }

    /** The name chosen at first start, else the model. */
    private fun deviceName(): String =
        getSharedPreferences("konnekta", MODE_PRIVATE).getString("deviceName", null)?.takeIf { it.isNotBlank() }
            ?: (Build.MODEL ?: "Android").filter { it.isLetterOrDigit() || it == ' ' || it == '-' }.take(32).ifBlank { "Android" }

    private fun run() {
        val core = try {
            val dl = getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS) ?: filesDir
            val cfg = JSONObject()
                .put("deviceName", deviceName())
                .put("deviceType", "phone")
                .put("downloadDir", dl.absolutePath)
                .put("callControl", true)
            Core.start(cfg, File(filesDir, "konnekta").apply { mkdirs() }.absolutePath)
        } catch (e: Throwable) {
            Log.e(TAG, "core failed to start", e)
            stopSelf()
            return
        }
        KonnektaApp.core = core
        refreshDevices(core)
        Streams.onChanged = { main.post { updateForeground() } }
        Streams.onRequestsChanged = { if (Streams.pending.value.isEmpty()) nm.cancel(N_STREAM) }
        main.post {
            registerReceiver(battery, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
            restartFeatures()
        }
        while (running) {
            val e = core.nextEvent(500) ?: continue
            try {
                handle(core, e)
            } catch (t: Throwable) {
                Log.w(TAG, "event handling failed", t)
            }
            KonnektaApp.publish(e)
        }
        // Stopped from this thread, after the last event call.
        KonnektaApp.core = null
        core.stop()
    }

    private fun handle(core: Core, e: JSONObject) {
        when (e.optString("event")) {
            "plugin" -> if (e.optString("kind") == "packet") {
                val data = e.optJSONObject("data") ?: return
                onPacket(core, e.optString("deviceId"), data)
            }
            "pairRequested" -> showPair(e, confirm = false)
            "pairConfirm" -> showPair(e, confirm = true)
            "paired", "pairRejected" -> {
                pairPrompt.value = null
                nm.cancel(N_PAIR)
            }
            "unpaired" -> ApproveKeys(this).remove(e.optString("deviceId"))
            "deviceDisconnected" -> Streams.stopAllFor(e.optString("deviceId"))
            "deviceConnected" -> {
                lastBattery?.let { b -> sendToAll(core, Types.BATTERY, b) }
                dnd?.sendState(e.optString("deviceId"))
            }
        }
        when (e.optString("event")) {
            "deviceConnected", "deviceDisconnected", "deviceUpdated", "paired", "unpaired", "pairRejected" -> refreshDevices(core)
        }
    }

    private fun refreshDevices(core: Core) {
        runCatching { core.callArray("devices") }.getOrNull()?.let { a ->
            Store.devices.value = (0 until a.length()).mapNotNull { a.optJSONObject(it) }
        }
    }

    private fun name(core: Core, id: String): String =
        runCatching { core.callObject("device", JSONObject().put("deviceId", id)).optString("name") }.getOrNull()
            ?.ifBlank { null } ?: "Computer"

    private fun onPacket(core: Core, deviceId: String, data: JSONObject) {
        val type = data.optString("type")
        val body = data.optJSONObject("body") ?: JSONObject()
        val payloadKey = data.optString("payloadKey").ifEmpty { null }
        when (type) {
            Types.PING -> {
                val msg = body.optString("message", "Ping")
                notify(N_PING, "Ping da ${name(core, deviceId)}", msg)
                Store.item(deviceId, "ping", "Ping", msg)
            }
            Types.CLIPBOARD -> body.optString("content").takeIf { it.isNotEmpty() }?.let { text ->
                main.post {
                    getSystemService(ClipboardManager::class.java)?.setPrimaryClip(ClipData.newPlainText("Konnekta", text))
                }
                Store.item(deviceId, "clipboard", "Appunti", text)
            }
            Types.FIND_DEVICE -> main.post {
                ring(name(core, deviceId))
                Store.item(deviceId, "ring", "${name(core, deviceId)} ti sta cercando")
            }
            Types.APPROVE_REQUEST -> onApproveRequest(core, deviceId, body)
            Types.SHARE -> onShare(core, deviceId, body, payloadKey)
            Types.BROWSE -> onBrowse(core, deviceId, body, payloadKey)
            Types.MEDIA -> {
                body.optJSONArray("playerList")?.let { l ->
                    Store.put(Store.pcPlayers, deviceId, (0 until l.length()).map { l.optString(it) })
                }
                val merged = JSONObject((Store.pcMedia.value[deviceId] ?: JSONObject()).toString())
                body.keys().forEach { k -> if (k != "playerList") merged.put(k, body.get(k)) }
                Store.put(Store.pcMedia, deviceId, merged)
            }
            Types.MIXER -> Store.put(Store.pcMixer, deviceId, body)
            Types.COMMANDS -> {
                val list = runCatching { JSONObject(body.optString("commandList", "{}")) }.getOrDefault(JSONObject())
                Store.put(Store.commands, deviceId, list.keys().asSequence().associateWith { list.getJSONObject(it) })
            }
            Types.DECK -> when (body.optString("type")) {
                "layout" -> Store.put(Store.deckLayout, deviceId, body)
                "state" -> {
                    val merged = JSONObject((Store.deckStates.value[deviceId] ?: JSONObject()).toString())
                    body.optJSONObject("states")?.let { st -> st.keys().forEach { merged.put(it, st.get(it)) } }
                    Store.put(Store.deckStates, deviceId, merged)
                }
            }
            Types.NOTIFICATION_REQUEST -> {
                if (body.optBoolean("request")) NotificationForwarder.resendAll(deviceId)
                body.optString("cancel").takeIf { it.isNotEmpty() }?.let { NotificationForwarder.dismiss(it) }
            }
            Types.NOTIFICATION_REPLY ->
                NotificationForwarder.reply(body.optString("requestReplyId"), body.optString("message"))
            Types.NOTIFICATION_ACTION ->
                NotificationForwarder.action(body.optString("key"), body.optString("action"))
            Types.MEDIA_REQUEST -> media?.handle(deviceId, body)
            Types.CALL_MUTE -> main.post { calls?.muteRinger() }
            Types.CALL_ACTION -> main.post { calls?.action(body.optString("action")) }
            Types.SMS_CONVERSATIONS_REQUEST -> sms?.conversations(deviceId)
            Types.SMS_THREAD_REQUEST -> sms?.thread(deviceId, body)
            Types.SMS_SEND -> sms?.send(body)
            Types.DND -> main.post { dnd?.apply(body) }
            Types.SCREENSHOT -> shots?.configure(deviceId, body)
            Types.PHOTO_REQUEST -> onPhotoRequest(core, deviceId, body.optString("mode", "photo"))
            Types.MICROPHONE, Types.CAMERA, Types.PHONE_SCREEN, Types.DESKTOP, Types.SYSTEM_AUDIO, Types.PHONE_AUDIO ->
                onStreamPacket(core, deviceId, type, body)
        }
    }

    // --- streams ---------------------------------------------------------------

    private fun foregroundTypes(vararg extra: String?): Int {
        var t = ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE
        val plugins = Streams.live.value.values.filter { it.outgoing }.map { it.plugin } + extra.filterNotNull()
        if ("microphone" in plugins) t = t or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
        if ("camera" in plugins) t = t or ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA
        if ("phonescreen" in plugins || "phoneaudio" in plugins) t = t or ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION
        if ("phoneaudio" in plugins) t = t or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
        return t
    }

    private fun serviceNotification(): Notification = NotificationCompat.Builder(this, KonnektaApp.CH_SERVICE)
        .setSmallIcon(R.drawable.ic_stat)
        .setContentTitle("Konnekta")
        .setContentText(
            Streams.live.value.values.filter { it.outgoing }.takeIf { it.isNotEmpty() }
                ?.joinToString(prefix = "In uso: ") { STREAM_NAMES[it.plugin] ?: it.plugin } ?: "Pronto a collegarsi ai tuoi computer",
        )
        .setContentIntent(openApp())
        .setOngoing(true)
        .build()

    /** Keep the foreground types and the notification in step with the streams. */
    private fun updateForeground() {
        runCatching { ServiceCompat.startForeground(this, N_SERVICE, serviceNotification(), foregroundTypes()) }
            .onFailure { Log.w(TAG, "foreground update failed", it) }
    }

    private fun onAcceptStream(i: Intent) {
        // A computer's request, or a stream the user starts from the phone.
        val r = Streams.pending.value.firstOrNull { it.deviceId == i.getStringExtra("device") && it.plugin == i.getStringExtra("plugin") }
            ?: StreamRequest(
                i.getStringExtra("device") ?: return, i.getStringExtra("name").orEmpty(), i.getStringExtra("plugin") ?: return,
                i.getStringExtra("session") ?: return, runCatching { JSONObject(i.getStringExtra("params") ?: "{}") }.getOrDefault(JSONObject()),
            )
        nm.cancel(N_STREAM)
        try {
            val audio = if (r.params.optBoolean("withAudio")) "phoneaudio" else null
            ServiceCompat.startForeground(this, N_SERVICE, serviceNotification(), foregroundTypes(r.plugin, audio))
        } catch (e: Exception) {
            Log.w(TAG, "cannot add ${r.plugin} to the foreground service", e)
            Streams.decline(r)
            return
        }
        @Suppress("DEPRECATION")
        val data: Intent? = if (Build.VERSION.SDK_INT >= 33) i.getParcelableExtra("projection", Intent::class.java) else i.getParcelableExtra("projection")
        val projection = data?.let {
            getSystemService(android.media.projection.MediaProjectionManager::class.java).getMediaProjection(i.getIntExtra("resultCode", 0), it)
        }
        val core = KonnektaApp.core ?: return
        transfers.execute { Streams.accept(this, core, r, projection) }
    }

    private fun onStreamPacket(core: Core, deviceId: String, type: String, body: JSONObject) {
        val plugin = Streams.TYPES.entries.firstOrNull { it.value == type }?.key ?: return
        val r = Streams.onPacket(this, core, deviceId, name(core, deviceId), plugin, body) ?: return
        val accept = appAction(80, Intent(this, MainActivity::class.java).setAction(MainActivity.ACTION_ACCEPT_STREAM)
            .putExtra("device", r.deviceId).putExtra("plugin", r.plugin))
        val deny = serviceAction(81, ACTION_DECLINE_STREAM) { it.putExtra("device", r.deviceId).putExtra("plugin", r.plugin) }
        notify(N_STREAM, "${r.deviceName} chiede ${STREAM_ASK[plugin]}", "Accetta o nega, oppure tocca per decidere nell'app",
            actions = listOf("Accetta" to accept, "Nega" to deny))
    }

    // --- photo / scan / QR on request -----------------------------------------

    private fun onPhotoRequest(core: Core, deviceId: String, mode: String) {
        val what = when (mode) {
            "scan" -> "di scansionare un testo"
            "qr" -> "di leggere un codice QR"
            else -> "una foto"
        }
        val from = name(core, deviceId)
        Store.dismissKind("photo")
        Store.item(deviceId, "photo", "$from chiede $what", target = mode)
        // Android does not let a background service open the camera: ask with a notification.
        val open = Intent(this, org.konnekta.app.ui.PhotoActivity::class.java)
            .putExtra(org.konnekta.app.ui.PhotoActivity.EXTRA_DEVICE, deviceId)
            .putExtra(org.konnekta.app.ui.PhotoActivity.EXTRA_MODE, mode)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        val deny = serviceAction(70, ACTION_DECLINE_PHOTO) { it.putExtra("device", deviceId) }
        notify(N_PHOTO, "$from chiede $what", "Tocca per aprire la fotocamera", open,
            actions = listOf("Apri fotocamera" to appAction(71, Intent(open)), "Nega" to deny))
    }

    // --- files and links -------------------------------------------------------

    private fun onShare(core: Core, deviceId: String, body: JSONObject, payloadKey: String?) {
        body.optString("text").takeIf { it.isNotEmpty() }?.let { text ->
            main.post {
                getSystemService(ClipboardManager::class.java)?.setPrimaryClip(ClipData.newPlainText("Konnekta", text))
            }
            Store.item(deviceId, "text", "Testo ricevuto", text)
            notify(N_SHARE, "Testo da ${name(core, deviceId)}", "Copiato negli appunti")
            return
        }
        body.optString("url").takeIf { it.isNotEmpty() }?.let { url ->
            Store.item(deviceId, "link", "Link ricevuto", url, target = url)
            notify(N_SHARE, "Link da ${name(core, deviceId)}", url, Intent(Intent.ACTION_VIEW, Uri.parse(url)))
            return
        }
        if (payloadKey != null) receiveFile(core, deviceId, payloadKey, body.optString("filename"))
    }

    private fun onBrowse(core: Core, deviceId: String, body: JSONObject, payloadKey: String?) {
        when (body.optString("action")) {
            "file" -> if (payloadKey != null) receiveFile(core, deviceId, payloadKey, body.optString("name"))
            "roots" -> body.optJSONArray("roots")?.let { Store.put(Store.browseRoots, deviceId, it) }
            else -> Store.put(Store.browse, deviceId, body)
        }
    }

    /** Downloads run off the event thread so other packets keep flowing. */
    private fun receiveFile(core: Core, deviceId: String, payloadKey: String, filename: String) {
        val t = Store.startTransfer("Ricevo ${Files.safeName(filename)}")
        transfers.execute {
            try {
                val (uri, fname) = Files.receive(this, core, payloadKey, filename)
                val mime = Files.mimeOf(fname)
                Store.item(deviceId, "file", fname, "Salvato in Download/${Files.FOLDER}", target = uri.toString(), mime = mime)
                notify(N_SHARE, "File da ${name(core, deviceId)}", fname,
                    Intent(Intent.ACTION_VIEW).setDataAndType(uri, mime).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION))
            } catch (e: Exception) {
                Log.w(TAG, "receive failed", e)
                Store.item(deviceId, "error", "Ricezione non riuscita", "${Files.safeName(filename)}: ${e.message}")
            } finally {
                Store.endTransfer(t)
            }
        }
    }

    // --- approvals -------------------------------------------------------------

    private fun replyApprove(core: Core, deviceId: String, id: String, error: String) {
        runCatching {
            core.sendPacket(deviceId, Types.APPROVE, JSONObject().put("id", id).put("result", "error").put("error", error))
        }
    }

    private fun onApproveRequest(core: Core, deviceId: String, body: JSONObject) {
        val id = body.optString("id")
        when (val kind = body.optString("kind")) {
            "cancel" -> {
                PendingApprovals.close(id)
                nm.cancel(N_APPROVE)
            }
            "approve", "enroll" -> {
                if (kind == "approve" && !ApproveKeys(this).hasKey(deviceId)) {
                    return replyApprove(core, deviceId, id, "this phone has no key for this computer: run sudo konnekta approve enroll")
                }
                if (PendingApprovals.current.value != null) {
                    return replyApprove(core, deviceId, id, "another request is open on the phone")
                }
                val computer = name(core, deviceId)
                PendingApprovals.show(PendingApprovals.Request(deviceId, computer, body))
                val timeout = body.optLong("timeout", 25).coerceIn(5, 120)
                main.postDelayed({
                    PendingApprovals.close(id)
                    nm.cancel(N_APPROVE)
                }, timeout * 1000)
                val open = PendingIntent.getActivity(
                    this, 0, Intent(this, ApproveActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                    PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
                )
                val title = if (kind == "enroll") "Registrare il telefono per sudo?" else "Approvare ${body.optString("service")}?"
                val n = NotificationCompat.Builder(this, KonnektaApp.CH_APPROVE)
                    .setSmallIcon(R.drawable.ic_stat)
                    .setContentTitle(title)
                    .setContentText("${body.optString("user")} su ${body.optString("host")}")
                    .setPriority(NotificationCompat.PRIORITY_HIGH)
                    .setCategory(NotificationCompat.CATEGORY_CALL)
                    .setContentIntent(open)
                    .setFullScreenIntent(open, true)
                    .setTimeoutAfter(timeout * 1000)
                    .setAutoCancel(true)
                    .build()
                nm.notify(N_APPROVE, n)
            }
        }
    }

    // --- pairing ---------------------------------------------------------------

    private fun showPair(e: JSONObject, confirm: Boolean) {
        pairPrompt.value = JSONObject(e.toString()).put("confirm", confirm)
        val id = e.optString("deviceId")
        val key = e.optString("verificationKey")
        fun answer(code: Int, accept: Boolean) = serviceAction(code, ACTION_PAIR_ANSWER) {
            it.putExtra("device", id).putExtra("key", key).putExtra("accept", accept)
        }
        notify(
            N_PAIR,
            if (confirm) "Conferma l'associazione" else "Richiesta di associazione",
            "${e.optString("deviceName")}: il computer deve mostrare ${formatKey(key)}",
            actions = listOf("Associa" to answer(50, true), "Rifiuta" to answer(51, false)),
        )
    }

    // --- battery ---------------------------------------------------------------

    private val battery = object : BroadcastReceiver() {
        override fun onReceive(c: Context, i: Intent) {
            val level = i.getIntExtra(BatteryManager.EXTRA_LEVEL, -1)
            val scale = i.getIntExtra(BatteryManager.EXTRA_SCALE, 100)
            val status = i.getIntExtra(BatteryManager.EXTRA_STATUS, -1)
            val charging = status == BatteryManager.BATTERY_STATUS_CHARGING || status == BatteryManager.BATTERY_STATUS_FULL
            val pct = if (level >= 0 && scale > 0) level * 100 / scale else -1
            val b = JSONObject().put("currentCharge", pct).put("isCharging", charging).put("thresholdEvent", 0)
            if (b.toString() == lastBattery?.toString()) return
            lastBattery = b
            val core = KonnektaApp.core ?: return
            io.execute { sendToAll(core, Types.BATTERY, b) }
        }
    }

    /** Send to every paired, connected device (the core checks permissions). */
    private fun sendToAll(core: Core, type: String, body: JSONObject) {
        val devices: JSONArray = runCatching { core.callArray("devices") }.getOrDefault(JSONArray())
        for (k in 0 until devices.length()) {
            val d = devices.optJSONObject(k) ?: continue
            if (d.optBoolean("paired") && d.optBoolean("connected")) {
                runCatching { core.sendPacket(d.optString("id"), type, body) }
            }
        }
    }

    // --- find my phone ---------------------------------------------------------

    private fun ring(from: String) {
        stopRing()
        val uri = RingtoneManager.getDefaultUri(RingtoneManager.TYPE_ALARM) ?: RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE)
        ringtone = RingtoneManager.getRingtone(this, uri)?.apply {
            audioAttributes = AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_ALARM).build()
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) isLooping = true
            play()
        }
        val stop = PendingIntent.getService(
            this, 1, Intent(this, KonnektaService::class.java).setAction(ACTION_STOP_RING), PendingIntent.FLAG_IMMUTABLE,
        )
        val n = NotificationCompat.Builder(this, KonnektaApp.CH_ALERTS)
            .setSmallIcon(R.drawable.ic_stat)
            .setContentTitle("$from ti sta cercando")
            .setContentText("Tocca per fermare la suoneria")
            .setContentIntent(stop)
            .addAction(0, "Ferma", stop)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .build()
        nm.notify(N_RING, n)
    }

    private fun stopRing() {
        Store.dismissKind("ring")
        ringtone?.stop()
        ringtone = null
        nm.cancel(N_RING)
    }

    // --- helpers ---------------------------------------------------------------

    private fun openApp(): PendingIntent = PendingIntent.getActivity(
        this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE,
    )

    private fun notify(id: Int, title: String, text: String, open: Intent? = null, actions: List<Pair<String, PendingIntent>> = emptyList()) {
        val tap = open?.let {
            PendingIntent.getActivity(this, id, it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        } ?: openApp()
        val b = NotificationCompat.Builder(this, KonnektaApp.CH_ALERTS)
            .setSmallIcon(R.drawable.ic_stat)
            .setContentTitle(title)
            .setContentText(text)
            .setContentIntent(tap)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
        actions.forEach { (label, pi) -> b.addAction(0, label, pi) }
        nm.notify(id, b.build())
    }

    /** Notification button handled by the service itself (no app on screen). */
    private fun serviceAction(code: Int, action: String, extras: (Intent) -> Unit): PendingIntent =
        PendingIntent.getService(this, code, Intent(this, KonnektaService::class.java).setAction(action).also(extras),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)

    /** Notification button that opens the app (Android permissions, screen-capture consent). */
    private fun appAction(code: Int, i: Intent): PendingIntent =
        PendingIntent.getActivity(this, code, i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)

    override fun onDestroy() {
        running = false
        stopRing()
        media?.stop(); calls?.stop(); sms?.stop(); dnd?.stop(); shots?.stop()
        Streams.stopAll()
        runCatching { unregisterReceiver(battery) }
        multicast?.release()
        io.shutdown()
        transfers.shutdownNow()
        super.onDestroy()
    }
}

val STREAM_NAMES = mapOf(
    "microphone" to "microfono", "camera" to "fotocamera", "phonescreen" to "schermo",
    "desktop" to "desktop remoto", "systemaudio" to "audio del computer", "phoneaudio" to "audio del telefono",
)
val STREAM_ASK = mapOf(
    "microphone" to "il microfono", "camera" to "la fotocamera come webcam", "phonescreen" to "lo schermo del telefono",
    "phoneaudio" to "l'audio del telefono",
)

/** "5EE6825F974ED59A" -> "5EE6 825F 974E D59A". */
fun formatKey(k: String): String = k.chunked(4).joinToString(" ")
