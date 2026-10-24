package org.konnekta.app.ui

import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Code
import androidx.compose.material.icons.filled.Fingerprint
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.SystemUpdate
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.json.JSONObject
import org.konnekta.app.service.Updates

/** Informazioni: version, identity of this phone, updates. */
@Composable
fun AboutScreen(onBack: () -> Unit) {
    val ctx = LocalContext.current
    val ui = LocalUi.current
    val scope = rememberCoroutineScope()
    var me by remember { mutableStateOf<JSONObject?>(null) }
    var status by remember { mutableStateOf<String?>(null) }
    val latest by Updates.latest.collectAsState()
    val progress by Updates.progress.collectAsState()
    LaunchedEffect(Unit) { me = core(ui.toast) { it.callObject("info") } }
    val current = Updates.current(ctx)
    fun check() = scope.launch {
        status = "Controllo in corso…"
        status = try {
            val r = Updates.check()
            when {
                r == null -> "Nessuna versione pubblicata su GitHub"
                Updates.newer(r.version, current) -> {
                    Updates.prompt.value = r
                    "Disponibile la versione ${r.version}"
                }
                else -> "Konnekta è aggiornato (versione $current)"
            }
        } catch (e: Exception) {
            "Controllo non riuscito: ${e.message ?: "nessuna connessione"}"
        }
    }

    SubHeader("Informazioni", null, onBack)
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
        ActionCard(Icons.Filled.Info, "Konnekta $current", "Collega telefono e computer in modo sicuro") {}
        me?.let { m ->
            SectionTitle("Questo telefono")
            ActionCard(Icons.Filled.Fingerprint, m.optString("deviceName"), "ID ${m.optString("deviceId")}") {}
            Text(m.optString("fingerprint"), style = MaterialTheme.typography.labelSmall.copy(fontFamily = FontFamily.Monospace),
                color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(horizontal = 4.dp))
            Hint("Impronta del certificato (SHA-256): è quella che il computer fissa all'associazione. " +
                "Protocollo Konnekta v${m.optInt("protocolVersion")}.")
        }
        SectionTitle("Aggiornamenti")
        ActionCard(
            Icons.Filled.SystemUpdate, "Cerca aggiornamenti",
            status ?: latest?.let { r -> if (Updates.newer(r.version, current)) "Disponibile la versione ${r.version}" else "Konnekta è aggiornato" }
                ?: "Konnekta controlla da solo a ogni avvio",
        ) { if (progress == null) check() }
        progress?.let { p ->
            LinearProgressIndicator(progress = { p }, modifier = Modifier.fillMaxWidth().padding(vertical = 8.dp))
            Hint("Download dell'aggiornamento: ${(p * 100).toInt()}%")
        }
        ActionCard(Icons.Filled.Code, "Codice e versioni", Updates.REPO.removePrefix("https://")) {
            ctx.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse("${Updates.REPO}/releases")))
        }
    }
}

/** "Nuova versione disponibile", over every screen (as on the desktop). */
@Composable
fun UpdateDialog() {
    val ctx = LocalContext.current
    val ui = LocalUi.current
    val scope = rememberCoroutineScope()
    val r by Updates.prompt.collectAsState()
    val progress by Updates.progress.collectAsState()
    val message by Updates.message.collectAsState()
    LaunchedEffect(message) {
        message?.let { ui.toast(it); Updates.message.value = null }
    }
    val rel = r ?: return
    AlertDialog(
        onDismissRequest = { if (progress == null) Updates.snooze(ctx, rel) },
        title = { Text("Nuova versione disponibile") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Konnekta ${rel.version} è disponibile (hai la ${Updates.current(ctx)}).")
                if (rel.notes.isNotBlank()) Text(rel.notes.take(1200),
                    Modifier.heightIn(max = 220.dp).verticalScroll(rememberScrollState()),
                    style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                if (rel.apkUrl == null) Text("Questa release non ha l'app Android: aprila per i dettagli.",
                    style = MaterialTheme.typography.bodySmall)
                progress?.let { p ->
                    LinearProgressIndicator(progress = { p }, modifier = Modifier.fillMaxWidth())
                    Text("Download ${(p * 100).toInt()}%… poi Android chiede di confermare l'installazione.",
                        style = MaterialTheme.typography.bodySmall)
                }
            }
        },
        confirmButton = {
            if (rel.apkUrl != null) Button(enabled = progress == null, onClick = { scope.launch { Updates.install(ctx, rel) } }) {
                Text("Aggiorna e installa")
            } else Button(onClick = {
                ctx.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(rel.page.ifEmpty { "${Updates.REPO}/releases" })))
                Updates.prompt.value = null
            }) { Text("Apri la pagina") }
        },
        dismissButton = {
            Row {
                TextButton(enabled = progress == null, onClick = { Updates.snooze(ctx, rel) }) { Text("Più tardi") }
            }
        },
    )
}
