package org.konnekta.app.ui

import android.os.SystemClock
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.core.Types
import org.konnekta.app.state.Store

/** Moves leave a Bluetooth link at most this often (40 per second). */
private const val BLUETOOTH_GAP_MS = 25L

/**
 * Send the touch input in [queue] to [deviceId], in order. Pointer and
 * scroll moves that pile up while one goes out are merged into one (summed,
 * or the latest position for "where you touch"), and on a Bluetooth link a
 * move leaves at most every 25 ms. Bluetooth carries fewer packets than a
 * finger makes: without this they queued up and the pointer trailed further
 * and further behind; now the computer always gets the latest move.
 * Clicks, keys and swipes are never merged or delayed.
 */
suspend fun sendInput(deviceId: String, queue: Channel<JSONObject>, type: String = Types.INPUT) = withContext(Dispatchers.IO) {
    var pending: JSONObject? = null
    var lastMove = 0L
    while (true) {
        var ev = pending ?: queue.receiveCatching().getOrNull() ?: break
        pending = null
        val kind = moveKind(ev)
        if (kind != null) {
            val bluetooth = Store.devices.value.firstOrNull { it.optString("id") == deviceId }?.optBoolean("bluetooth") == true
            val wait = lastMove + (if (bluetooth) BLUETOOTH_GAP_MS else 0L) - SystemClock.uptimeMillis()
            if (wait > 0) delay(wait)
            while (true) {
                val next = queue.tryReceive().getOrNull() ?: break
                if (moveKind(next) == kind) ev = merge(kind, ev, next)
                else {
                    pending = next
                    break
                }
            }
            lastMove = SystemClock.uptimeMillis()
        }
        runCatching { KonnektaApp.core?.sendPacket(deviceId, type, ev) }
    }
}

private enum class Move { Relative, Scroll, Absolute }

private val RELATIVE = setOf("dx", "dy")
private val SCROLL = setOf("dx", "dy", "scroll")
private val ABSOLUTE = setOf("konnektaAbsX", "konnektaAbsY")
private val ABSOLUTE_EXTEND = setOf("konnektaAbsX", "konnektaAbsY", "konnektaScreen")

/** The kind of a plain move (nothing else in the packet), or null. */
private fun moveKind(b: JSONObject): Move? {
    val keys = b.keys().asSequence().toSet()
    return when (keys) {
        RELATIVE -> Move.Relative
        SCROLL -> Move.Scroll
        ABSOLUTE, ABSOLUTE_EXTEND -> Move.Absolute
        else -> null
    }
}

private fun merge(kind: Move, a: JSONObject, b: JSONObject): JSONObject = when (kind) {
    Move.Absolute -> b
    else -> JSONObject(a.toString())
        .put("dx", a.optDouble("dx") + b.optDouble("dx"))
        .put("dy", a.optDouble("dy") + b.optDouble("dy"))
}
