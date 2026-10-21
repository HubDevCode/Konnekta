package org.konnekta.app.ui

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import android.widget.Toast
import androidx.activity.ComponentActivity
import org.konnekta.app.KonnektaApp
import org.konnekta.app.service.ClipboardSync
import org.konnekta.app.service.KonnektaService

/**
 * Sends the phone clipboard to the computers from anywhere: an invisible
 * window that takes focus (Android lets only a focused app read the
 * clipboard), sends and closes. Opened by the Quick Settings tile, the
 * Konnekta notification, and "Invia al computer" on selected text.
 */
class ClipboardSendActivity : ComponentActivity() {
    private var done = false
    private val main = Handler(Looper.getMainLooper())

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        KonnektaService.start(this)
        // Selected text comes with the intent: no clipboard needed.
        intent.getCharSequenceExtra(Intent.EXTRA_PROCESS_TEXT)?.toString()?.takeIf { it.isNotBlank() }?.let {
            done = true
            send(applicationContext, it)
            finish()
            return
        }
        // Rarely the window never gets focus: give up instead of waiting.
        main.postDelayed({
            if (!done) {
                done = true
                toast(applicationContext, "Non riesco a leggere gli appunti, riprova")
                finish()
            }
        }, 2_000)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus || done) return
        done = true
        main.removeCallbacksAndMessages(null)
        val text = ClipboardSync.read(this)
        if (text == null) toast(applicationContext, "Gli appunti sono vuoti") else send(applicationContext, text)
        finish()
    }

    override fun onDestroy() {
        main.removeCallbacksAndMessages(null)
        super.onDestroy()
    }

    private fun send(app: Context, text: String) = Thread {
        // Right after a cold start the core may still be connecting.
        var n = 0
        repeat(10) {
            if (KonnektaApp.core != null) {
                n = ClipboardSync.sendToAll(text)
                if (n > 0) return@repeat
            }
            Thread.sleep(500)
        }
        toast(app, if (n > 0) "Appunti inviati" else "Nessun computer collegato con il permesso Appunti")
    }.start()

    companion object {
        private fun toast(app: Context, msg: String) =
            Handler(Looper.getMainLooper()).post { Toast.makeText(app, msg, Toast.LENGTH_SHORT).show() }

        fun intent(ctx: Context): Intent = Intent(ctx, ClipboardSendActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_ANIMATION)

        fun pending(ctx: Context): PendingIntent =
            PendingIntent.getActivity(ctx, 90, intent(ctx), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }
}

/** Quick Settings tile "Invia appunti". */
class ClipboardTileService : TileService() {
    override fun onStartListening() {
        super.onStartListening()
        qsTile?.apply {
            state = Tile.STATE_ACTIVE
            label = "Invia appunti"
            updateTile()
        }
    }

    override fun onClick() {
        super.onClick()
        if (Build.VERSION.SDK_INT >= 34) {
            startActivityAndCollapse(ClipboardSendActivity.pending(this))
        } else {
            @Suppress("DEPRECATION", "StartActivityAndCollapseDeprecated")
            startActivityAndCollapse(ClipboardSendActivity.intent(this))
        }
    }
}
