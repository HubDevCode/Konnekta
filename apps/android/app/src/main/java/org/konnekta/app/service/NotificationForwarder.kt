package org.konnekta.app.service

import android.app.Notification
import android.app.RemoteInput
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.provider.Settings
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.util.Log
import org.json.JSONArray
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.core.Types
import java.util.concurrent.Executors

/**
 * Forwards the phone's notifications to the paired computers and applies
 * what they ask back: dismiss, reply, press an action. The core sends only
 * to computers with the Notifications permission.
 */
class NotificationForwarder : NotificationListenerService() {
    companion object {
        private const val TAG = "Konnekta"

        @Volatile
        private var instance: NotificationForwarder? = null
        private val io = Executors.newSingleThreadExecutor()

        fun enabled(ctx: Context): Boolean =
            Settings.Secure.getString(ctx.contentResolver, "enabled_notification_listeners")
                ?.contains(ComponentName(ctx, NotificationForwarder::class.java).flattenToString()) == true

        fun settingsIntent(): Intent = Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)

        /** A computer asked for every current notification. */
        fun resendAll(deviceId: String) = instance?.let { s -> io.execute { s.sendAll(deviceId) } }

        fun dismiss(key: String) = instance?.let { s -> runCatching { s.cancelNotification(key) } }

        fun reply(key: String, message: String) = instance?.reply(key, message)

        fun action(key: String, title: String) = instance?.action(key, title)
    }

    override fun onListenerConnected() {
        instance = this
    }

    override fun onListenerDisconnected() {
        if (instance === this) instance = null
    }

    override fun onNotificationPosted(sbn: StatusBarNotification) {
        val body = toBody(sbn) ?: return
        io.execute { broadcast(body) }
    }

    override fun onNotificationRemoved(sbn: StatusBarNotification) {
        if (!forwardable(sbn)) return
        io.execute { broadcast(JSONObject().put("id", sbn.key).put("isCancel", true)) }
    }

    private fun forwardable(sbn: StatusBarNotification): Boolean {
        val n = sbn.notification
        return sbn.packageName != packageName &&
            n.flags and Notification.FLAG_GROUP_SUMMARY == 0 &&
            n.flags and Notification.FLAG_FOREGROUND_SERVICE == 0 &&
            (sbn.isClearable || n.category == Notification.CATEGORY_CALL || n.category == Notification.CATEGORY_MESSAGE)
    }

    private fun appName(pkg: String): String = runCatching {
        packageManager.getApplicationLabel(packageManager.getApplicationInfo(pkg, 0)).toString()
    }.getOrDefault(pkg)

    private fun toBody(sbn: StatusBarNotification): JSONObject? {
        if (!forwardable(sbn)) return null
        val n = sbn.notification
        val x: Bundle = n.extras
        val title = x.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty()
        val text = (x.getCharSequence(Notification.EXTRA_BIG_TEXT) ?: x.getCharSequence(Notification.EXTRA_TEXT))?.toString().orEmpty()
        if (title.isEmpty() && text.isEmpty()) return null
        val actions = JSONArray()
        var replyable = false
        n.actions?.forEach { a ->
            if (a.remoteInputs?.isNotEmpty() == true) replyable = true else a.title?.let { actions.put(it.toString()) }
        }
        return JSONObject()
            .put("id", sbn.key)
            .put("appName", appName(sbn.packageName))
            .put("title", title)
            .put("text", text)
            .put("ticker", n.tickerText?.toString() ?: "$title: $text")
            .put("isClearable", sbn.isClearable)
            // Updates of an alert-once notification (progress, chats) must not pop up again.
            .put("silent", n.flags and Notification.FLAG_ONLY_ALERT_ONCE != 0)
            .apply {
                if (actions.length() > 0) put("actions", actions)
                if (replyable) put("requestReplyId", sbn.key)
            }
    }

    private fun broadcast(body: JSONObject) {
        val core = KonnektaApp.core ?: return
        for (d in connectedPaired()) runCatching { core.sendPacket(d, Types.NOTIFICATION, body) }
    }

    private fun sendAll(deviceId: String) {
        val core = KonnektaApp.core ?: return
        val active = runCatching { activeNotifications }.getOrNull() ?: return
        for (sbn in active) {
            val b = toBody(sbn)?.put("silent", true) ?: continue
            runCatching { core.sendPacket(deviceId, Types.NOTIFICATION, b) }
        }
    }

    private fun find(key: String): StatusBarNotification? =
        runCatching { activeNotifications }.getOrNull()?.firstOrNull { it.key == key }

    private fun reply(key: String, message: String) {
        val sbn = find(key) ?: return
        val a = sbn.notification.actions?.firstOrNull { it.remoteInputs?.isNotEmpty() == true } ?: return
        val intent = Intent()
        val results = Bundle()
        for (ri in a.remoteInputs) results.putCharSequence(ri.resultKey, message)
        RemoteInput.addResultsToIntent(a.remoteInputs, intent, results)
        runCatching { a.actionIntent.send(this, 0, intent) }.onFailure { Log.w(TAG, "reply failed", it) }
    }

    private fun action(key: String, title: String) {
        val sbn = find(key) ?: return
        val a = sbn.notification.actions?.firstOrNull { it.title?.toString() == title } ?: return
        runCatching { a.actionIntent.send() }.onFailure { Log.w(TAG, "action failed", it) }
    }
}

/** Ids of paired, connected computers. */
fun connectedPaired(): List<String> {
    val core = KonnektaApp.core ?: return emptyList()
    val list = runCatching { core.callArray("devices") }.getOrNull() ?: return emptyList()
    return (0 until list.length()).mapNotNull { list.optJSONObject(it) }
        .filter { it.optBoolean("paired") && it.optBoolean("connected") }
        .map { it.optString("id") }
}
