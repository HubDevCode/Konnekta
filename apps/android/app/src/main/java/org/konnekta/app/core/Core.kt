package org.konnekta.app.core

import com.sun.jna.Library
import com.sun.jna.Native
import com.sun.jna.Pointer
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.locks.ReentrantReadWriteLock
import kotlin.concurrent.read
import kotlin.concurrent.write

/** The C ABI of the Rust core, see crates/konnekta-ffi/include/konnekta.h. */
@Suppress("FunctionName")
internal interface KonnektaLib : Library {
    fun konnekta_start(configJson: String, dataDir: String): Pointer?
    fun konnekta_call(h: Pointer, method: String, paramsJson: String): Pointer?
    fun konnekta_next_event(h: Pointer, timeoutMs: Int): Pointer?
    fun konnekta_free_string(s: Pointer)
    fun konnekta_stop(h: Pointer)
}

class CoreException(message: String) : Exception(message)

/**
 * The Konnekta core running in this process. Networking, TLS, pairing and
 * permission checks all happen in Rust; this class only passes JSON.
 * Calls block: use them from a background thread.
 */
class Core private constructor(private val handle: Pointer) {
    @Volatile
    private var stopped = false

    /** Calls hold the read lock; [stop] takes the write lock, so the native
     *  handle is never freed while a call is running on another thread. */
    private val lock = ReentrantReadWriteLock()

    companion object {
        private val lib: KonnektaLib by lazy { Native.load("konnekta_ffi", KonnektaLib::class.java) }

        fun start(config: JSONObject, dataDir: String): Core {
            val h = lib.konnekta_start(config.toString(), dataDir)
                ?: throw CoreException("the core did not start (see logcat, tag Konnekta)")
            return Core(h)
        }
    }

    private fun take(p: Pointer): String = try {
        p.getString(0, "UTF-8")
    } finally {
        lib.konnekta_free_string(p)
    }

    /** One API call; the `result` value, or [CoreException] with the core's error. */
    fun call(method: String, params: JSONObject? = null): Any? {
        val p = lock.read {
            if (stopped) throw CoreException("core stopped")
            lib.konnekta_call(handle, method, params?.toString() ?: "")
        } ?: throw CoreException("no answer from the core")
        val o = JSONObject(take(p))
        if (o.has("error")) throw CoreException(o.getString("error"))
        return o.opt("result").takeUnless { it == JSONObject.NULL }
    }

    fun callObject(method: String, params: JSONObject? = null): JSONObject =
        call(method, params) as? JSONObject ?: JSONObject()

    fun callArray(method: String, params: JSONObject? = null): JSONArray =
        call(method, params) as? JSONArray ?: JSONArray()

    /** Next core event, or null after [timeoutMs]. */
    fun nextEvent(timeoutMs: Int): JSONObject? {
        val p = lock.read {
            if (stopped) return null
            lib.konnekta_next_event(handle, timeoutMs)
        } ?: return null
        return runCatching { JSONObject(take(p)) }.getOrNull()
    }

    /** Send a packet on behalf of a phone plugin (permission-checked in Rust). */
    fun sendPacket(deviceId: String, type: String, body: JSONObject) {
        call("sendPacket", JSONObject().put("deviceId", deviceId).put("type", type).put("body", body))
    }

    fun stop() = lock.write {
        if (!stopped) {
            stopped = true
            lib.konnekta_stop(handle)
        }
    }
}

/** Packet types used by the app (same names as crates/konnekta-proto/src/types.rs). */
object Types {
    const val PING = "konnekta.ping"
    const val BATTERY = "konnekta.battery"
    const val FIND_DEVICE = "konnekta.finddevice"
    const val CLIPBOARD = "konnekta.clipboard"
    const val APPROVE = "konnekta.approve"
    const val APPROVE_REQUEST = "konnekta.approve.request"
    const val SHARE = "konnekta.share"
    const val BROWSE = "konnekta.browse"
    const val BROWSE_REQUEST = "konnekta.browse.request"
    const val MEDIA = "konnekta.media"
    const val MEDIA_REQUEST = "konnekta.media.request"
    const val INPUT = "konnekta.input"
    const val PRESENTER = "konnekta.presenter"
    const val COMMANDS = "konnekta.commands"
    const val COMMANDS_REQUEST = "konnekta.commands.request"
    const val DECK = "konnekta.deck"
    const val DECK_REQUEST = "konnekta.deck.request"
    const val NOTIFICATION = "konnekta.notification"
    const val NOTIFICATION_REQUEST = "konnekta.notification.request"
    const val NOTIFICATION_REPLY = "konnekta.notification.reply"
    const val NOTIFICATION_ACTION = "konnekta.notification.action"
}
