package org.konnekta.app.service

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.database.ContentObserver
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Telephony
import android.telephony.SmsManager
import android.util.Log
import org.json.JSONArray
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.core.Types
import java.util.concurrent.Executors

/**
 * Text messages for the computers, with KDE Connect field names
 * (`_id`, `thread_id`, `addresses`, `type` 1 received / 2 sent).
 */
class Sms(private val ctx: Context) {
    companion object {
        const val TAG = "Konnekta"
        val PERMISSIONS = arrayOf(Manifest.permission.READ_SMS, Manifest.permission.SEND_SMS)
        private const val MAX_PAGE = 100
    }

    private val io = Executors.newSingleThreadExecutor()
    private var lastSeen = 0L
    private val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
        override fun onChange(selfChange: Boolean) {
            io.execute { pushNew() }
        }
    }

    private fun canRead() = ctx.checkSelfPermission(Manifest.permission.READ_SMS) == PackageManager.PERMISSION_GRANTED

    fun start() {
        if (!canRead()) return
        io.execute { lastSeen = newestDate() }
        runCatching { ctx.contentResolver.registerContentObserver(Telephony.Sms.CONTENT_URI, true, observer) }
    }

    fun stop() {
        runCatching { ctx.contentResolver.unregisterContentObserver(observer) }
        io.shutdown()
    }

    private val columns = arrayOf(
        Telephony.Sms._ID, Telephony.Sms.THREAD_ID, Telephony.Sms.ADDRESS, Telephony.Sms.BODY,
        Telephony.Sms.DATE, Telephony.Sms.TYPE, Telephony.Sms.READ,
    )

    private fun query(selection: String?, args: Array<String>?, limit: Int): List<JSONObject> {
        if (!canRead()) return emptyList()
        val out = ArrayList<JSONObject>()
        runCatching {
            ctx.contentResolver.query(Telephony.Sms.CONTENT_URI, columns, selection, args, "${Telephony.Sms.DATE} DESC")?.use { c ->
                while (c.moveToNext() && out.size < limit) {
                    out += JSONObject()
                        .put("_id", c.getLong(0))
                        .put("thread_id", c.getLong(1))
                        .put("addresses", JSONArray().put(JSONObject().put("address", c.getString(2).orEmpty())))
                        .put("body", c.getString(3).orEmpty())
                        .put("date", c.getLong(4))
                        .put("type", c.getInt(5))
                        .put("read", c.getInt(6))
                }
            }
        }.onFailure { Log.w(TAG, "sms query failed", it) }
        return out
    }

    private fun newestDate(): Long = query(null, null, 1).firstOrNull()?.optLong("date") ?: 0L

    private fun reply(deviceId: String, msgs: List<JSONObject>) {
        val body = JSONObject().put("messages", JSONArray(msgs)).put("version", 2)
        runCatching { KonnektaApp.core?.sendPacket(deviceId, Types.SMS_MESSAGES, body) }
    }

    /** Latest message of every conversation. */
    fun conversations(deviceId: String) = io.execute {
        val seen = HashSet<Long>()
        val latest = query(null, null, 5000).filter { seen.add(it.optLong("thread_id")) }.take(300)
        reply(deviceId, latest)
    }

    /** One conversation, newest first, optionally older than a timestamp. */
    fun thread(deviceId: String, b: JSONObject) = io.execute {
        val id = b.optLong("threadID")
        val n = b.optInt("numberToRequest", 50).coerceIn(1, MAX_PAGE)
        val msgs = if (b.has("rangeStartTimestamp")) {
            query("${Telephony.Sms.THREAD_ID} = ? AND ${Telephony.Sms.DATE} < ?", arrayOf("$id", "${b.optLong("rangeStartTimestamp")}"), n)
        } else {
            query("${Telephony.Sms.THREAD_ID} = ?", arrayOf("$id"), n)
        }
        reply(deviceId, msgs)
    }

    /** konnekta.sms.send: {addresses:[{address}], messageBody}. */
    fun send(b: JSONObject) = io.execute {
        val text = b.optString("messageBody").take(5000)
        val to = b.optJSONArray("addresses")?.let { a -> (0 until a.length()).mapNotNull { a.optJSONObject(it)?.optString("address") } }
            .orEmpty().filter { it.isNotBlank() }.take(20)
        if (text.isEmpty() || to.isEmpty()) return@execute
        if (ctx.checkSelfPermission(Manifest.permission.SEND_SMS) != PackageManager.PERMISSION_GRANTED) return@execute
        runCatching {
            @Suppress("DEPRECATION")
            val sm = if (Build.VERSION.SDK_INT >= 31) ctx.getSystemService(SmsManager::class.java) else SmsManager.getDefault()
            for (addr in to) sm.sendMultipartTextMessage(addr, null, sm.divideMessage(text), null, null)
        }.onFailure { Log.w(TAG, "sms send failed", it) }
    }

    /** New messages (received or sent from the phone) go to every computer. */
    private fun pushNew() {
        val fresh = query("${Telephony.Sms.DATE} > ?", arrayOf("$lastSeen"), 50)
        if (fresh.isEmpty()) return
        lastSeen = fresh.maxOf { it.optLong("date") }
        for (d in connectedPaired()) reply(d, fresh)
    }
}
