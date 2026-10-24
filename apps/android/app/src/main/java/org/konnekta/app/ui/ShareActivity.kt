package org.konnekta.app.ui

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.util.Patterns
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Column
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.core.Types
import org.konnekta.app.service.KonnektaService
import org.konnekta.app.state.Store

/** "Share › Konnekta" from other apps: text, links and files to a computer. */
class ShareActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Store.load(this)
        KonnektaService.start(this)
        val ui = Ui(::toast) { null }
        var choices by mutableStateOf<List<JSONObject>?>(null)
        setContent {
            KonnektaTheme {
                CompositionLocalProvider(LocalUi provides ui) {
                    choices?.let { list ->
                        AlertDialog(
                            onDismissRequest = ::finish,
                            title = { Text("Invia a") },
                            text = {
                                Column {
                                    list.forEach { d ->
                                        ActionCard(Icons.Filled.Computer, d.optString("name"), TRANSPORT_LABELS[d.optString("transport")] ?: d.optString("address")) {
                                            choices = null
                                            Store.select(this@ShareActivity, d.optString("id"))
                                            deliver(ui, d.optString("id"))
                                        }
                                    }
                                }
                            },
                            confirmButton = {},
                            dismissButton = { TextButton(onClick = ::finish) { Text("Annulla") } },
                        )
                    }
                }
            }
        }
        lifecycleScope.launch {
            val online = waitForComputers()
            when {
                online.isEmpty() -> {
                    toast("Nessun computer collegato")
                    finish()
                }
                online.size == 1 -> deliver(ui, online.first().optString("id"))
                else -> choices = online
            }
        }
    }

    /** The core may still be starting when we are opened from another app. */
    private suspend fun waitForComputers(): List<JSONObject> = withContext(Dispatchers.IO) {
        repeat(30) {
            val core = KonnektaApp.core
            if (core != null) {
                val list = runCatching { core.callArray("devices") }.getOrNull()?.objects().orEmpty()
                    .filter { it.optBoolean("paired") && it.optBoolean("connected") }
                if (list.isNotEmpty() || it > 15) return@withContext list
            }
            delay(200)
        }
        emptyList()
    }

    private fun deliver(ui: Ui, deviceId: String) {
        val i = intent
        val uris: List<Uri> = when (i.action) {
            Intent.ACTION_SEND -> listOfNotNull(stream(i))
            Intent.ACTION_SEND_MULTIPLE -> streams(i)
            else -> emptyList()
        }
        if (uris.isNotEmpty()) {
            sendUris(this, ui, deviceId, uris)
            finish()
            return
        }
        val text = i.getStringExtra(Intent.EXTRA_TEXT)?.trim().orEmpty()
        if (text.isEmpty()) return finish()
        val body = if (Patterns.WEB_URL.matcher(text).matches() && text.contains("://")) JSONObject().put("url", text)
        else JSONObject().put("text", text)
        lifecycleScope.launch {
            core(ui.toast) { it.sendPacket(deviceId, Types.SHARE, body) }?.let { toast("Inviato a ${Store.name(deviceId)}") }
            finish()
        }
    }

    @Suppress("DEPRECATION")
    private fun stream(i: Intent): Uri? =
        if (Build.VERSION.SDK_INT >= 33) i.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java) else i.getParcelableExtra(Intent.EXTRA_STREAM)

    @Suppress("DEPRECATION")
    private fun streams(i: Intent): List<Uri> =
        (if (Build.VERSION.SDK_INT >= 33) i.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java)
        else i.getParcelableArrayListExtra(Intent.EXTRA_STREAM)).orEmpty()

    private fun toast(msg: String) = Toast.makeText(applicationContext, msg, Toast.LENGTH_SHORT).show()
}
