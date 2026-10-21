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
    fun konnekta_stream_write(h: Pointer, channel: Long, kind: Byte, flags: Byte, ptsUs: Long, data: ByteArray, len: Long): Int
    fun konnekta_stream_read(h: Pointer, channel: Long, buf: ByteArray, cap: Long, meta: ByteArray, ptsUs: LongArray, timeoutMs: Int): Long
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

    /**
     * Write one frame on a stream channel (see [Frame]): 0 sent, 1 dropped
     * (peer not connected yet, or too slow), -1 channel gone.
     */
    fun streamWrite(channel: Long, kind: Int, flags: Int, ptsUs: Long, data: ByteArray, len: Int = data.size): Int = lock.read {
        if (stopped) return -1
        lib.konnekta_stream_write(handle, channel, kind.toByte(), flags.toByte(), ptsUs, data, len.toLong())
    }

    /**
     * Read the next frame into [buf]: its length, 0 on timeout, -1 when the
     * channel closed. [meta] gets kind and flags, [pts] the timestamp.
     */
    fun streamRead(channel: Long, buf: ByteArray, meta: ByteArray, pts: LongArray, timeoutMs: Int): Int = lock.read {
        if (stopped) return -1
        lib.konnekta_stream_read(handle, channel, buf, buf.size.toLong(), meta, pts, timeoutMs).toInt()
    }

    fun stop() = lock.write {
        if (!stopped) {
            stopped = true
            lib.konnekta_stop(handle)
        }
    }
}

/** Stream frame kinds and flags (crates/konnekta-ffi/include/konnekta.h). */
object Frame {
    const val AUDIO = 1
    const val VIDEO = 2
    const val CONTROL = 3
    const val KEYFRAME = 1
    const val CONFIG = 2
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
    const val CALL = "konnekta.call"
    const val CALL_MUTE = "konnekta.call.mute"
    const val CALL_ACTION = "konnekta.call.action"
    const val SMS_MESSAGES = "konnekta.sms.messages"
    const val SMS_SEND = "konnekta.sms.send"
    const val SMS_CONVERSATIONS_REQUEST = "konnekta.sms.conversations.request"
    const val SMS_THREAD_REQUEST = "konnekta.sms.thread.request"
    const val DND = "konnekta.dnd"
    const val SCREENSHOT = "konnekta.screenshot"
    const val PHOTO = "konnekta.photo"
    const val PHOTO_REQUEST = "konnekta.photo.request"
    const val MICROPHONE = "konnekta.microphone"
    const val CAMERA = "konnekta.camera"
    const val PHONE_SCREEN = "konnekta.phonescreen"
    const val DESKTOP = "konnekta.desktop"
    const val SYSTEM_AUDIO = "konnekta.systemaudio"
}
