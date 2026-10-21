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
        const val ACTION_STOP_RING = "org.konnekta.app.STOP_RING"

        /** Pairing request waiting for the user: {deviceId, deviceName, verificationKey, confirm}. */
        val pairPrompt = MutableStateFlow<JSONObject?>(null)

        fun start(context: Context) {
            context.startForegroundService(Intent(context, KonnektaService::class.java))
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
        return START_STICKY
    }

    private fun deviceName(): String =
        (Build.MODEL ?: "Android").filter { it.isLetterOrDigit() || it == ' ' || it == '-' }.take(32).ifBlank { "Android" }

    private fun run() {
        val core = try {
            val dl = getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS) ?: filesDir
            val cfg = JSONObject()
                .put("deviceName", deviceName())
                .put("deviceType", "phone")
                .put("downloadDir", dl.absolutePath)
                .put("callControl", false)
            Core.start(cfg, File(filesDir, "konnekta").apply { mkdirs() }.absolutePath)
        } catch (e: Throwable) {
            Log.e(TAG, "core failed to start", e)
            stopSelf()
            return
        }
        KonnektaApp.core = core
        refreshDevices(core)
        main.post { registerReceiver(battery, IntentFilter(Intent.ACTION_BATTERY_CHANGED)) }
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
            "deviceConnected" -> lastBattery?.let { b -> sendToAll(core, Types.BATTERY, b) }
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
        }
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
        notify(
            N_PAIR,
            if (confirm) "Conferma l'associazione" else "Richiesta di associazione",
            "${e.optString("deviceName")}: confronta il codice ${formatKey(e.optString("verificationKey"))}",
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

    private fun notify(id: Int, title: String, text: String, open: Intent? = null) {
        val tap = open?.let {
            PendingIntent.getActivity(this, id, it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        } ?: openApp()
        val n: Notification = NotificationCompat.Builder(this, KonnektaApp.CH_ALERTS)
            .setSmallIcon(R.drawable.ic_stat)
            .setContentTitle(title)
            .setContentText(text)
            .setContentIntent(tap)
            .setAutoCancel(true)
            .build()
        nm.notify(id, n)
    }

    override fun onDestroy() {
        running = false
        stopRing()
        runCatching { unregisterReceiver(battery) }
        multicast?.release()
        io.shutdown()
        transfers.shutdownNow()
        super.onDestroy()
    }
}

/** "5EE6825F974ED59A" -> "5EE6 825F 974E D59A". */
fun formatKey(k: String): String = k.chunked(4).joinToString(" ")
