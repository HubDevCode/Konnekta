package org.konnekta.app.ui

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material3.Button
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.json.JSONObject
import org.konnekta.app.approve.ApproveActivity
import org.konnekta.app.approve.PendingApprovals
import org.konnekta.app.core.Types
import org.konnekta.app.service.Files
import org.konnekta.app.service.KonnektaService
import org.konnekta.app.state.InboxItem
import org.konnekta.app.state.Store
import java.text.DateFormat
import java.util.Date

@Composable
fun InboxScreen(current: JSONObject?, items: List<InboxItem>, open: (Route) -> Unit) {
    val ctx = LocalContext.current
    val ui = LocalUi.current
    val scope = rememberCoroutineScope()
    val approval by PendingApprovals.current.collectAsState()
    val media by Store.pcMedia.collectAsState()
    val transfers by Store.transfers.collectAsState()
    val ring = items.firstOrNull { it.kind == "ring" }
    val rest = items.filter { it.kind != "ring" }
    val playing = current?.let { media[it.optString("id")] }?.takeIf { it.optString("title").isNotEmpty() }
    val urgent = listOfNotNull(approval, ring).size

    LazyColumn(Modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        item {
            val n = urgent + rest.size
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(vertical = 6.dp)) {
                StatusDot(urgent == 0)
                Spacer(Modifier.width(8.dp))
                Text(
                    when {
                        urgent > 0 -> if (urgent == 1) "1 richiesta ti aspetta" else "$urgent richieste ti aspettano"
                        n == 0 -> "Niente di nuovo"
                        else -> "$n elementi"
                    },
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
        }
        approval?.let { a ->
            item {
                MasterTile(
                    label = if (a.kind == "enroll") "Registrazione" else "Da approvare",
                    meta = a.body.optString("service"),
                    title = if (a.kind == "enroll") "Registrare il telefono per sudo" else "Approva ${a.body.optString("service")}",
                    text = "${a.body.optString("user")} su ${a.body.optString("host")} · ${a.deviceName}",
                    color = LocalStatus.current.warn,
                ) {
                    Button(onClick = {
                        ctx.startActivity(Intent(ctx, ApproveActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                    }) { Text("Rivedi") }
                }
            }
        }
        ring?.let { r ->
            item {
                MasterTile("Suoneria", r.deviceName, r.title, "Il telefono sta suonando a tutto volume.", LocalStatus.current.danger) {
                    Button(onClick = { KonnektaService.stopRinging(ctx) }) { Text("L'ho trovato") }
                }
            }
        }
        if (transfers.isNotEmpty()) {
            items(transfers.values.toList()) { label -> SmallCard("In corso", MaterialTheme.colorScheme.secondary, label, "", null) }
        }
        playing?.let { m ->
            item {
                val id = current!!.optString("id")
                val isPlaying = m.optBoolean("isPlaying")
                SmallCard(
                    if (isPlaying) "In riproduzione" else "In pausa",
                    LocalStatus.current.ok,
                    m.optString("title"),
                    listOf(m.optString("artist"), m.optString("player")).filter { it.isNotEmpty() }.joinToString(" · "),
                    onClick = { open(Route.Media) },
                ) {
                    FilledTonalIconButton(onClick = {
                        scope.launch { send(ui.toast, id, Types.MEDIA_REQUEST, JSONObject().put("action", "PlayPause").apply {
                            m.optString("player").takeIf { it.isNotEmpty() }?.let { put("player", it) }
                        }) }
                    }) { Icon(if (isPlaying) Icons.Filled.Pause else Icons.Filled.PlayArrow, null) }
                }
            }
        }
        items(rest, key = { it.id }) { it -> InboxCard(it) }
        if (rest.isEmpty() && urgent == 0 && playing == null) {
            item {
                Hint(
                    if (current == null) "Associa un computer dalla scheda Computer per iniziare."
                    else "Qui compaiono appunti, file e link che arrivano da ${current.optString("name")}, e le richieste che hanno bisogno di te.",
                    Modifier.padding(top = 24.dp),
                )
            }
        }
    }
}

@Composable
private fun MasterTile(label: String, meta: String, title: String, text: String, color: Color, action: @Composable () -> Unit) {
    Surface(
        shape = RoundedCornerShape(16.dp),
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        border = BorderStroke(1.5.dp, MaterialTheme.colorScheme.primary),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Pill(label, color)
                if (meta.isNotEmpty()) {
                    Spacer(Modifier.width(8.dp))
                    Text(meta, style = MaterialTheme.typography.labelSmall.copy(fontFamily = FontFamily.Monospace),
                        color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            Text(title, style = MaterialTheme.typography.titleLarge)
            Text(text, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) { action() }
        }
    }
}

@Composable
private fun SmallCard(
    label: String,
    color: Color,
    title: String,
    text: String,
    onDismiss: (() -> Unit)? = null,
    onClick: (() -> Unit)? = null,
    action: (@Composable () -> Unit)? = null,
) {
    Surface(
        onClick = onClick ?: {},
        enabled = onClick != null,
        shape = RoundedCornerShape(14.dp),
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        border = BorderStroke(1.dp, LocalStatus.current.border),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(Modifier.padding(start = 14.dp, top = 10.dp, bottom = 10.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Pill(label, color)
                Text(title, style = MaterialTheme.typography.titleSmall, maxLines = 3, overflow = TextOverflow.Ellipsis)
                if (text.isNotEmpty()) Text(text, style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 2, overflow = TextOverflow.Ellipsis)
            }
            action?.invoke()
            if (onDismiss != null) IconButton(onClick = onDismiss) { Icon(Icons.Filled.Close, "Rimuovi") }
        }
    }
}

@Composable
private fun InboxCard(item: InboxItem) {
    val ctx = LocalContext.current
    val ui = LocalUi.current
    val s = LocalStatus.current
    val scheme = MaterialTheme.colorScheme
    val time = DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(item.at))
    val (label, color) = when (item.kind) {
        "clipboard" -> "Appunti" to scheme.primary
        "text" -> "Testo" to scheme.primary
        "link" -> "Link" to scheme.secondary
        "file" -> "File ricevuto" to s.ok
        "ping" -> "Ping" to scheme.secondary
        "error" -> "Errore" to s.danger
        else -> item.kind to scheme.primary
    }
    val body = when (item.kind) {
        "file", "error" -> item.text
        else -> "Da ${item.deviceName} · $time"
    }
    val title = if (item.kind in setOf("clipboard", "text", "link")) item.text else item.title
    SmallCard("$label · $time", color, title, if (item.kind in setOf("clipboard", "text", "link")) "Da ${item.deviceName}" else body,
        onDismiss = { Store.dismiss(item.id) },
        onClick = when (item.kind) {
            "file" -> item.target?.let { t -> { Files.open(ctx, Uri.parse(t), item.mime) } }
            "link" -> item.target?.let { t -> { runCatching { ctx.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(t)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) } } }
            "clipboard", "text" -> { {
                ctx.getSystemService(ClipboardManager::class.java)?.setPrimaryClip(ClipData.newPlainText("Konnekta", item.text))
                ui.toast("Copiato negli appunti")
            } }
            else -> null
        },
    ) {
        when (item.kind) {
            "file" -> OutlinedButton(onClick = { item.target?.let { Files.open(ctx, Uri.parse(it), item.mime) } }) { Text("Apri") }
            "link" -> OutlinedButton(onClick = {
                runCatching { ctx.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(item.target)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
            }) { Text("Apri") }
        }
    }
}
