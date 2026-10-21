package org.konnekta.app.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.core.Core
import org.konnekta.app.state.Store
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.KeyboardArrowUp
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material3.Button
import androidx.compose.material3.TextButton
import org.konnekta.app.state.Settings

/** Run a core call off the main thread; errors become a toast. */
suspend fun <T> core(toast: (String) -> Unit, f: (Core) -> T): T? =
    withContext(Dispatchers.IO) {
        val c = KonnektaApp.core ?: return@withContext null
        try {
            f(c)
        } catch (e: Exception) {
            withContext(Dispatchers.Main) { toast(e.message ?: "errore") }
            null
        }
    }

/** Fire-and-forget packet to a computer (permission-checked in the core). */
suspend fun send(toast: (String) -> Unit, deviceId: String, type: String, body: JSONObject) {
    core(toast) { it.sendPacket(deviceId, type, body) }
}

fun JSONArray.objects(): List<JSONObject> = (0 until length()).mapNotNull { optJSONObject(it) }

/** Italian names of the permissions a computer can be given. */
val PERMISSION_LABELS = mapOf(
    "ping" to "Ping",
    "battery" to "Stato della batteria",
    "clipboard" to "Appunti",
    "files" to "Trasferimento file",
    "notifications" to "Notifiche",
    "telephony" to "Chiamate",
    "mediaControl" to "Controllo multimediale",
    "presenter" to "Telecomando per presentazioni",
    "remoteInput" to "Mouse e tastiera",
    "commands" to "Esecuzione di comandi",
    "deck" to "Azioni del Mobile Deck",
    "microphone" to "Microfono",
    "camera" to "Fotocamera e webcam",
    "desktop" to "Desktop remoto",
    "systemAudio" to "Audio di sistema",
    "doNotDisturb" to "Sincronizzazione Non disturbare",
    "screenshots" to "Screenshot",
    "findMyPhone" to "Trova il dispositivo",
    "browseFiles" to "Sfoglia i file del computer",
    "messages" to "SMS",
    "approve" to "Approvazione con impronta (sudo)",
)

@Composable
fun StatusDot(on: Boolean, modifier: Modifier = Modifier) {
    val s = LocalStatus.current
    Box(modifier.size(8.dp).clip(CircleShape).background(if (on) s.ok else s.muted.copy(alpha = .5f)))
}

/** Scope chip: which computer the screens act on. */
@Composable
fun ScopeChip(devices: List<JSONObject>, current: JSONObject?, compact: Boolean = false) {
    val ctx = LocalContext.current
    var open by remember { mutableStateOf(false) }
    val paired = devices.filter { it.optBoolean("paired") }
    Box {
        Surface(
            onClick = { if (paired.size > 1) open = true },
            shape = RoundedCornerShape(12.dp),
            color = MaterialTheme.colorScheme.surfaceContainerHigh,
        ) {
            Row(Modifier.padding(horizontal = if (compact) 8.dp else 12.dp, vertical = if (compact) 3.dp else 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Filled.Computer, null, Modifier.size(if (compact) 14.dp else 18.dp), tint = MaterialTheme.colorScheme.primary)
                Spacer(Modifier.width(if (compact) 6.dp else 8.dp))
                Text(current?.optString("name") ?: "Nessun computer",
                    style = if (compact) MaterialTheme.typography.labelMedium else MaterialTheme.typography.titleSmall)
                Spacer(Modifier.width(8.dp))
                paired.forEach { StatusDot(it.optBoolean("connected"), Modifier.padding(end = 3.dp)) }
                if (paired.size > 1) Icon(Icons.Filled.ExpandMore, null, Modifier.size(18.dp))
            }
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            paired.forEach { d ->
                DropdownMenuItem(
                    text = { Text(d.optString("name")) },
                    leadingIcon = { StatusDot(d.optBoolean("connected")) },
                    onClick = {
                        Store.select(ctx, d.optString("id"))
                        open = false
                    },
                )
            }
        }
    }
}

@Composable
fun SectionTitle(text: String) {
    Text(text, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(top = 16.dp, bottom = 6.dp))
}

/** Flux-style row: icon, title, one line of explanation. */
@Composable
fun ActionCard(
    icon: ImageVector,
    title: String,
    subtitle: String,
    enabled: Boolean = true,
    highlight: Boolean = false,
    trailing: @Composable (() -> Unit)? = null,
    onClick: () -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    Surface(
        onClick = onClick,
        enabled = enabled,
        shape = RoundedCornerShape(14.dp),
        color = if (highlight) scheme.primaryContainer else scheme.surfaceContainerLow,
        border = BorderStroke(1.dp, LocalStatus.current.border),
        modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp),
    ) {
        Row(Modifier.padding(14.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(icon, null, tint = if (enabled) scheme.primary else scheme.onSurfaceVariant.copy(alpha = .5f))
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Text(title, style = MaterialTheme.typography.titleMedium,
                    color = if (enabled) scheme.onSurface else scheme.onSurfaceVariant.copy(alpha = .6f))
                Text(subtitle, style = MaterialTheme.typography.bodySmall, color = scheme.onSurfaceVariant,
                    maxLines = 2, overflow = TextOverflow.Ellipsis)
            }
            trailing?.invoke()
        }
    }
}

/** Header of a screen opened from a tab: back, title, computer name. */
@Composable
fun SubHeader(title: String, subtitle: String?, onBack: () -> Unit, actions: @Composable () -> Unit = {}) {
    Row(Modifier.fillMaxWidth().padding(vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        Surface(onClick = onBack, shape = RoundedCornerShape(12.dp), color = MaterialTheme.colorScheme.surfaceContainerHigh) {
            Icon(Icons.AutoMirrored.Filled.ArrowBack, "Indietro", Modifier.padding(10.dp))
        }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleLarge)
            if (subtitle != null) Text(subtitle, style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
                color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        actions()
    }
}

/** The 16-character key in four blocks, as on the computer. */
@Composable
fun KeyBlocks(key: String) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        key.chunked(4).forEach { part ->
            Surface(
                shape = RoundedCornerShape(10.dp),
                color = MaterialTheme.colorScheme.surfaceContainerHighest,
                border = BorderStroke(1.dp, LocalStatus.current.border),
                modifier = Modifier.weight(1f),
            ) {
                Text(part, Modifier.padding(vertical = 12.dp).fillMaxWidth(),
                    style = MaterialTheme.typography.titleMedium.copy(fontFamily = FontFamily.Monospace),
                    color = MaterialTheme.colorScheme.primary, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
            }
        }
    }
}

@Composable
fun Hint(text: String, modifier: Modifier = Modifier) {
    Text(text, modifier.padding(vertical = 8.dp), style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant)
}

@Composable
fun Pill(text: String, color: Color) {
    Text(text, Modifier.clip(RoundedCornerShape(99.dp)).background(color.copy(alpha = .14f)).padding(horizontal = 8.dp, vertical = 2.dp),
        style = MaterialTheme.typography.labelSmall, color = color)
}

@Composable
fun BackButton(onBack: () -> Unit) = IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Indietro") }

/** One feature button; shown as a row or as a tile depending on the setting. */
data class Fn(
    val icon: ImageVector,
    val title: String,
    val subtitle: String,
    val enabled: Boolean = true,
    val highlight: Boolean = false,
    val trailing: (@Composable () -> Unit)? = null,
    val onClick: () -> Unit,
) {
    /** Stable id for the saved order: the title, which never names a device. */
    val key: String get() = title
}

/** A titled group of features on Controllo or Invia. */
class FnSection(val key: String, val title: String?, val items: List<Fn>)

/**
 * The feature sections of [screen] in the user's order, without the hidden
 * ones. In "Personalizza" mode every section and feature gets arrows to
 * move it and an eye to hide or show it.
 */
@Composable
fun FnSections(screen: String, sections: List<FnSection>) {
    val ctx = LocalContext.current
    val editing by Settings.editing.collectAsState()
    val hidden by Settings.hidden.collectAsState()
    @Suppress("UNUSED_VARIABLE") val order by Settings.order.collectAsState()
    val arranged = Settings.arrange(screen, sections) { it.key }
    if (editing != screen) {
        arranged.forEach { sec ->
            val shown = Settings.arrange("$screen/${sec.key}", sec.items) { it.key }.filter { "$screen/${it.key}" !in hidden }
            if (shown.isEmpty()) return@forEach
            sec.title?.let { SectionTitle(it) }
            FnList(shown)
        }
        return
    }
    val secKeys = arranged.map { it.key }
    Row(Modifier.fillMaxWidth().padding(top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        Text("Personalizza: sposta con le frecce, nascondi con l'occhio", Modifier.weight(1f),
            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        TextButton(onClick = { Settings.resetLayout(ctx, screen) }) { Text("Ripristina") }
        Button(onClick = { Settings.editing.value = null }) { Text("Fine") }
    }
    arranged.forEach { sec ->
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(sec.title ?: "In alto", Modifier.weight(1f).padding(top = 16.dp, bottom = 6.dp),
                style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)
            if (arranged.size > 1) {
                Arrows(secKeys, sec.key) { by -> Settings.move(ctx, screen, secKeys, sec.key, by) }
            }
        }
        val items = Settings.arrange("$screen/${sec.key}", sec.items) { it.key }
        val keys = items.map { it.key }
        items.forEach { f ->
            val k = "$screen/${f.key}"
            val off = k in hidden
            Surface(
                shape = RoundedCornerShape(12.dp),
                color = MaterialTheme.colorScheme.surfaceContainerLow,
                border = BorderStroke(1.dp, LocalStatus.current.border),
                modifier = Modifier.fillMaxWidth().padding(vertical = 3.dp),
            ) {
                Row(Modifier.padding(start = 12.dp, end = 4.dp, top = 4.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                    Icon(f.icon, null, tint = if (off) MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = .4f) else MaterialTheme.colorScheme.primary)
                    Spacer(Modifier.width(12.dp))
                    Text(f.title, Modifier.weight(1f), style = MaterialTheme.typography.titleSmall,
                        color = if (off) MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = .6f) else MaterialTheme.colorScheme.onSurface)
                    IconButton(onClick = { Settings.setHidden(ctx, k, !off) }) {
                        Icon(if (off) Icons.Filled.VisibilityOff else Icons.Filled.Visibility, if (off) "Mostra" else "Nascondi")
                    }
                    Arrows(keys, f.key) { by -> Settings.move(ctx, "$screen/${sec.key}", keys, f.key, by) }
                }
            }
        }
    }
}

@Composable
private fun Arrows(keys: List<String>, key: String, move: (Int) -> Unit) {
    val i = keys.indexOf(key)
    IconButton(onClick = { move(-1) }, enabled = i > 0) { Icon(Icons.Filled.KeyboardArrowUp, "Su") }
    IconButton(onClick = { move(1) }, enabled = i < keys.size - 1) { Icon(Icons.Filled.KeyboardArrowDown, "Giù") }
}

/** Entry to "Personalizza" at the end of Controllo and Invia. */
@Composable
fun CustomizeButton(screen: String) {
    val editing by Settings.editing.collectAsState()
    if (editing == screen) return
    TextButton(onClick = { Settings.editing.value = screen }, modifier = Modifier.padding(vertical = 4.dp)) {
        Icon(Icons.Filled.Edit, null, Modifier.size(18.dp))
        Spacer(Modifier.width(6.dp))
        Text("Personalizza questa pagina")
    }
}

/** A group of features: one per line, or two per row with the grid setting. */
@Composable
fun FnList(items: List<Fn>) {
    val grid by Store.grid.collectAsState()
    if (!grid) {
        items.forEach { ActionCard(it.icon, it.title, it.subtitle, it.enabled, it.highlight, it.trailing, it.onClick) }
        return
    }
    items.chunked(2).forEach { row ->
        Row(Modifier.fillMaxWidth().height(androidx.compose.foundation.layout.IntrinsicSize.Max), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            row.forEach { FnTile(it, Modifier.weight(1f).fillMaxHeight()) }
            if (row.size == 1) Spacer(Modifier.weight(1f))
        }
    }
}

@Composable
private fun FnTile(f: Fn, modifier: Modifier) {
    val scheme = MaterialTheme.colorScheme
    Surface(
        onClick = f.onClick,
        enabled = f.enabled,
        shape = RoundedCornerShape(14.dp),
        color = if (f.highlight) scheme.primaryContainer else scheme.surfaceContainerLow,
        border = BorderStroke(1.dp, LocalStatus.current.border),
        modifier = modifier.padding(vertical = 4.dp),
    ) {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Icon(f.icon, null, tint = if (f.enabled) scheme.primary else scheme.onSurfaceVariant.copy(alpha = .5f))
            Text(f.title, style = MaterialTheme.typography.titleSmall,
                color = if (f.enabled) scheme.onSurface else scheme.onSurfaceVariant.copy(alpha = .6f))
            Text(f.subtitle, style = MaterialTheme.typography.bodySmall, color = scheme.onSurfaceVariant,
                maxLines = 3, overflow = TextOverflow.Ellipsis)
            f.trailing?.invoke()
        }
    }
}
