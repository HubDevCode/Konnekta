package org.konnekta.konnekta

import android.app.Notification
import android.app.PendingIntent
import android.app.RemoteInput
import android.content.Intent
import android.os.Bundle
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification

/**
 * Mirrors notifications to paired computers (`konnekta.notification`
 * packets) and executes dismiss / reply / action requests coming back.
 */
class KonnektaNotificationListener : NotificationListenerService() {
    companion object {
        @Volatile var instance: KonnektaNotificationListener? = null
        private val replies = HashMap<String, Pair<Notification.Action, RemoteInput>>()
        private val actions = HashMap<String, Map<String, PendingIntent>>()
    }

    override fun onListenerConnected() { instance = this }
    override fun onListenerDisconnected() { instance = null }

    private fun keep(sbn: StatusBarNotification): Boolean {
        val n = sbn.notification
        if (sbn.packageName == packageName) return false
        if (n.flags and Notification.FLAG_GROUP_SUMMARY != 0) return false
        if (n.flags and Notification.FLAG_FOREGROUND_SERVICE != 0) return false
        return n.extras.getCharSequence(Notification.EXTRA_TITLE) != null || n.extras.getCharSequence(Notification.EXTRA_TEXT) != null
    }

    private fun appName(pkg: String): String = try {
        packageManager.getApplicationLabel(packageManager.getApplicationInfo(pkg, 0)).toString()
    } catch (_: Exception) { pkg }

    fun send(sbn: StatusBarNotification) {
        if (!keep(sbn)) return
        val n = sbn.notification
        val ex: Bundle = n.extras
        val body = HashMap<String, Any?>()
        body["id"] = sbn.key
        body["appName"] = appName(sbn.packageName)
        body["title"] = ex.getCharSequence(Notification.EXTRA_TITLE)?.toString() ?: ""
        body["text"] = (ex.getCharSequence(Notification.EXTRA_BIG_TEXT) ?: ex.getCharSequence(Notification.EXTRA_TEXT))?.toString() ?: ""
        body["ticker"] = n.tickerText?.toString()
        body["isClearable"] = sbn.isClearable
        body["silent"] = n.priority < Notification.PRIORITY_DEFAULT
        val labels = ArrayList<String>()
        val pending = HashMap<String, PendingIntent>()
        n.actions?.forEach { a ->
            val ri = a.remoteInputs?.firstOrNull { it.allowFreeFormInput }
            if (ri != null) {
                val replyId = sbn.key + "#reply"
                replies[replyId] = a to ri
                body["requestReplyId"] = replyId
            } else if (a.title != null && a.actionIntent != null) {
                labels.add(a.title.toString())
                pending[a.title.toString()] = a.actionIntent
            }
        }
        if (labels.isNotEmpty()) body["actions"] = labels
        actions[sbn.key] = pending
        Bridge.emit(mapOf("type" to "notification", "body" to body))
    }

    override fun onNotificationPosted(sbn: StatusBarNotification) = send(sbn)

    override fun onNotificationRemoved(sbn: StatusBarNotification) {
        actions.remove(sbn.key)
        replies.remove(sbn.key + "#reply")
        Bridge.emit(mapOf("type" to "notification", "body" to mapOf("id" to sbn.key, "isCancel" to true)))
    }

    fun resendAll() = activeNotifications?.forEach { send(it) }

    fun dismiss(key: String) = cancelNotification(key)

    fun reply(replyId: String, message: String): Boolean {
        val (action, input) = replies[replyId] ?: return false
        val intent = Intent()
        val results = Bundle().apply { putCharSequence(input.resultKey, message) }
        RemoteInput.addResultsToIntent(arrayOf(input), intent, results)
        action.actionIntent.send(this, 0, intent)
        return true
    }

    fun action(key: String, label: String): Boolean {
        val pi = actions[key]?.get(label) ?: return false
        pi.send()
        return true
    }
}
