package org.konnekta.app.service

import android.app.NotificationManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.provider.Settings
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.core.Types
import java.util.concurrent.Executors

/**
 * Do Not Disturb both ways. The phone reports its own changes; a computer's
 * change is applied only with "Do Not Disturb access" granted by the user.
 */
class Dnd(private val ctx: Context) {
    private val nm = ctx.getSystemService(NotificationManager::class.java)
    private val io = Executors.newSingleThreadExecutor()
    /** State we set ourselves: its broadcast must not bounce back. */
    @Volatile
    private var applying: Boolean? = null
    /** Android repeats the broadcast: report each state once. */
    private var reported: Boolean? = null

    companion object {
        fun accessIntent() = Intent(Settings.ACTION_NOTIFICATION_POLICY_ACCESS_SETTINGS)
        fun hasAccess(ctx: Context) = ctx.getSystemService(NotificationManager::class.java).isNotificationPolicyAccessGranted
    }

    private fun enabled() = nm.currentInterruptionFilter.let {
        it != NotificationManager.INTERRUPTION_FILTER_ALL && it != NotificationManager.INTERRUPTION_FILTER_UNKNOWN
    }

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(c: Context, i: Intent) {
            val on = enabled()
            if (on == reported) return
            reported = on
            if (applying == on) {
                applying = null
                return
            }
            broadcast(on, userInitiated = true)
        }
    }

    fun start() {
        reported = enabled()
        ctx.registerReceiver(receiver, IntentFilter(NotificationManager.ACTION_INTERRUPTION_FILTER_CHANGED))
    }

    fun stop() {
        runCatching { ctx.unregisterReceiver(receiver) }
        io.shutdown()
    }

    private fun body(on: Boolean, userInitiated: Boolean) =
        JSONObject().put("enabled", on).put("changedAt", System.currentTimeMillis()).put("userInitiated", userInitiated)

    private fun broadcast(on: Boolean, userInitiated: Boolean) = io.execute {
        val b = body(on, userInitiated)
        for (d in connectedPaired()) runCatching { KonnektaApp.core?.sendPacket(d, Types.DND, b) }
    }

    /** Tell a computer that just connected where we stand. */
    fun sendState(deviceId: String) = io.execute {
        runCatching { KonnektaApp.core?.sendPacket(deviceId, Types.DND, body(enabled(), userInitiated = false)) }
    }

    /** konnekta.dnd from a computer. */
    fun apply(b: JSONObject) {
        val on = b.optBoolean("enabled")
        if (on == enabled() || !nm.isNotificationPolicyAccessGranted) return
        applying = on
        runCatching {
            nm.setInterruptionFilter(if (on) NotificationManager.INTERRUPTION_FILTER_PRIORITY else NotificationManager.INTERRUPTION_FILTER_ALL)
        }.onFailure { applying = null }
    }
}
