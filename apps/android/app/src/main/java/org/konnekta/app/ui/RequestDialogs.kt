package org.konnekta.app.ui

import android.content.Intent
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.platform.LocalContext
import kotlinx.coroutines.flow.MutableStateFlow
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.approve.ApproveActivity
import org.konnekta.app.approve.PendingApprovals
import org.konnekta.app.core.Types
import org.konnekta.app.service.KonnektaService
import org.konnekta.app.service.STREAM_ASK
import org.konnekta.app.service.Streams
import org.konnekta.app.state.Store

/** "Accetta" pressed in a stream-request notification: the app finishes the start. */
object NotificationAccept {
    val stream = MutableStateFlow<Pair<String, String>?>(null) // deviceId, plugin
}

/**
 * What the computers ask, as dialogs over any screen: one at a time,
 * the most urgent first. The same requests have Accept / Deny in their
 * notifications.
 */
@Composable
fun RequestDialogs() {
    val ctx = LocalContext.current
    val asks by Streams.pending.collectAsState()
    val items by Store.inbox.collectAsState()
    val approval by PendingApprovals.current.collectAsState()
    val fromNotification by NotificationAccept.stream.collectAsState()
    val start = rememberStreamStarter()

    // Accepted from the notification: run the same start (permissions, consent).
    LaunchedEffect(fromNotification, asks) {
        val (dev, plugin) = fromNotification ?: return@LaunchedEffect
        asks.firstOrNull { it.deviceId == dev && it.plugin == plugin }?.let {
            NotificationAccept.stream.value = null
            start(it)
        }
    }

    val ring = items.firstOrNull { it.kind == "ring" }
    val photo = items.firstOrNull { it.kind == "photo" }
    val ask = asks.firstOrNull()
    when {
        ring != null -> AlertDialog(
            onDismissRequest = {},
            title = { Text(ring.title) },
            text = { Text("Il telefono sta suonando a tutto volume.") },
            confirmButton = { Button(onClick = { KonnektaService.stopRinging(ctx) }) { Text("L'ho trovato") } },
        )
        approval != null -> AlertDialog(
            onDismissRequest = {},
            title = { Text(if (approval!!.kind == "enroll") "Registrare il telefono per sudo?" else "Approvare ${approval!!.body.optString("service")}?") },
            text = { Text("${approval!!.body.optString("user")} su ${approval!!.body.optString("host")} · ${approval!!.deviceName}") },
            confirmButton = {
                Button(onClick = {
                    ctx.startActivity(Intent(ctx, ApproveActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                }) { Text("Rivedi") }
            },
        )
        ask != null -> AlertDialog(
            onDismissRequest = {},
            title = { Text("${ask.deviceName} chiede ${STREAM_ASK[ask.plugin]}") },
            text = {
                Text(when (ask.plugin) {
                    "microphone" -> "Il computer userà il telefono come microfono finché non lo fermi."
                    "camera" -> "Il computer vedrà la fotocamera come una webcam finché non la fermi."
                    "phoneaudio" -> "Il computer riprodurrà l'audio del telefono finché non lo fermi."
                    else -> "Il computer vedrà tutto ciò che appare sullo schermo del telefono."
                })
            },
            confirmButton = { Button(onClick = { start(ask) }) { Text("Accetta") } },
            dismissButton = { TextButton(onClick = { Streams.decline(ask) }) { Text("Nega") } },
        )
        photo != null -> AlertDialog(
            onDismissRequest = {},
            title = { Text(photo.title) },
            text = { Text("La foto o il testo vanno solo a ${photo.deviceName}.") },
            confirmButton = {
                Button(onClick = {
                    ctx.startActivity(Intent(ctx, PhotoActivity::class.java)
                        .putExtra(PhotoActivity.EXTRA_DEVICE, photo.deviceId)
                        .putExtra(PhotoActivity.EXTRA_MODE, photo.target ?: "photo"))
                }) { Text("Apri fotocamera") }
            },
            dismissButton = {
                TextButton(onClick = {
                    Store.dismiss(photo.id)
                    ctx.getSystemService(android.app.NotificationManager::class.java).cancel(KonnektaService.N_PHOTO)
                    Thread { runCatching { KonnektaApp.core?.sendPacket(photo.deviceId, Types.PHOTO, JSONObject().put("cancelled", true)) } }.start()
                }) { Text("Nega") }
            },
        )
    }
}
