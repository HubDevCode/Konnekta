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
fun SmallCard(
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
fun InboxCard(item: InboxItem) {
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
            "file" -> item.target?.let { t -> { Files.open(ctx, Uri.parse(t), item.mime)?.let(ui.toast) } }
            "link" -> item.target?.let { t -> { runCatching { ctx.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(t)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) } } }
            "clipboard", "text" -> { {
                ctx.getSystemService(ClipboardManager::class.java)?.setPrimaryClip(ClipData.newPlainText("Konnekta", item.text))
                ui.toast("Copiato negli appunti")
            } }
            else -> null
        },
    ) {
        when (item.kind) {
            "file" -> OutlinedButton(onClick = { item.target?.let { Files.open(ctx, Uri.parse(it), item.mime)?.let(ui.toast) } }) { Text("Apri") }
            "link" -> OutlinedButton(onClick = {
                runCatching { ctx.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(item.target)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
            }) { Text("Apri") }
        }
    }
}
