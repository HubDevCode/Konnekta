package org.konnekta.app.service

import android.app.Activity
import android.app.Application
import android.content.ClipboardManager
import android.content.Context
import android.os.Bundle
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.core.Types
import org.konnekta.app.state.Store

/**
 * Phone clipboard to the computers. Android 10+ lets an app read the
 * clipboard only while one of its windows has focus, so (as in Flux):
 * * while Konnekta is on screen, every copy goes out by itself;
 * * elsewhere, the "Invia appunti" Quick Settings tile, the button in the
 *   Konnekta notification and "Invia al computer" in the text selection menu
 *   open [org.konnekta.app.ui.ClipboardSendActivity], an invisible window
 *   that reads the clipboard, sends it and closes.
 */
object ClipboardSync {
    /** Last text the computer put on the phone: never sent back. */
    @Volatile
    var fromComputer: String? = null
    @Volatile
    private var lastSent: String? = null

    private var started = 0
    private var cm: ClipboardManager? = null
    private val listener = ClipboardManager.OnPrimaryClipChangedListener { onCopy() }

    /** Watch the clipboard while any Konnekta window is visible. */
    fun install(app: Application) {
        cm = app.getSystemService(ClipboardManager::class.java)
        app.registerActivityLifecycleCallbacks(object : Application.ActivityLifecycleCallbacks {
            override fun onActivityStarted(a: Activity) {
                if (started++ == 0) cm?.addPrimaryClipChangedListener(listener)
            }

            override fun onActivityStopped(a: Activity) {
                if (--started == 0) cm?.removePrimaryClipChangedListener(listener)
            }

            override fun onActivityCreated(a: Activity, b: Bundle?) {}
            override fun onActivityResumed(a: Activity) {}
            override fun onActivityPaused(a: Activity) {}
            override fun onActivitySaveInstanceState(a: Activity, b: Bundle) {}
            override fun onActivityDestroyed(a: Activity) {}
        })
    }

    private fun onCopy() {
        val text = read(cm) ?: return
        if (text == fromComputer || text == lastSent) return
        Thread { sendToAll(text) }.start()
    }

    fun read(cm: ClipboardManager?): String? = runCatching {
        cm?.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.text?.toString()
    }.getOrNull()?.takeIf { it.isNotEmpty() }

    fun read(ctx: Context): String? = read(ctx.getSystemService(ClipboardManager::class.java))

    /**
     * Send [text] to every connected paired computer that allows the
     * clipboard. Returns how many got it. Blocking: call off the main thread.
     */
    fun sendToAll(text: String): Int {
        val core = KonnektaApp.core ?: return 0
        lastSent = text
        val body = JSONObject().put("content", text.take(MAX_CHARS))
        val devices = runCatching { core.callArray("devices") }.getOrNull()
            ?.let { a -> (0 until a.length()).mapNotNull { a.optJSONObject(it) } }
            ?: Store.paired
        return devices.filter { it.optBoolean("paired") && it.optBoolean("connected") }.count { d ->
            runCatching { core.sendPacket(d.optString("id"), Types.CLIPBOARD, body) }.isSuccess
        }
    }

    /** Large enough for any real copy, small enough for one packet. */
    private const val MAX_CHARS = 200_000
}
