package org.konnekta.app.ui

import android.provider.OpenableColumns
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.InsertDriveFile
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.AdsClick
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.filled.ContentPaste
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Devices
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.FolderOpen
import androidx.compose.material.icons.filled.Image
import androidx.compose.material.icons.filled.Language
import androidx.compose.material.icons.filled.Laptop
import androidx.compose.material.icons.filled.Movie
import androidx.compose.material.icons.filled.MusicNote
import androidx.compose.material.icons.filled.NotificationsActive
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Smartphone
import androidx.compose.material.icons.filled.Sync
import androidx.compose.material.icons.filled.Tablet
import androidx.compose.material.icons.filled.TextFields
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.GlobalScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject
import org.konnekta.app.core.Types
import org.konnekta.app.service.Files
import org.konnekta.app.state.Store

/** Something picked to send: files, photos and videos, a text, the clipboard. */
data class Pick(
    val kind: String,
    val uri: android.net.Uri? = null,
    val name: String,
    val size: Long = 0,
    val mime: String = "",
    val text: String? = null,
)

/** The Invia page state held by the process: the selection survives tab swipes and rotation. */
object SendNav {
    val receive = MutableStateFlow(false)
    val selection = MutableStateFlow<List<Pick>>(emptyList())
}

private fun pickOf(ctx: android.content.Context, uri: android.net.Uri): Pick {
    var name = "file"
    var size = 0L
    runCatching {
        ctx.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { c ->
            if (c.moveToFirst()) {
                c.getString(0)?.let { name = it }
                if (!c.isNull(1)) size = c.getLong(1)
            }
        }
    }
    val mime = runCatching { ctx.contentResolver.getType(uri) }.getOrNull() ?: Files.mimeOf(name)
    return Pick("file", uri, name, size, mime)
}

/** LocalSend status, devices and transfers from the core (events keep them fresh after). */
suspend fun refreshLocalSend() {
    core({}) { c ->
        Store.lsStatus.value = c.pluginAction("localsend", "status") as? JSONObject
        (c.pluginAction("localsend", "peers") as? JSONArray)?.let { Store.lsPeers.value = it.objects() }
        (c.pluginAction("localsend", "transfers") as? JSONArray)?.let { a ->
            Store.lsTransfers.value = a.objects().associateBy { it.optString("id") }
        }
    }
}

/**
 * Invia, laid out like LocalSend: "Invia" (what to send, then the device
 * to send it to: computers with Konnekta and devices with LocalSend) and
 * "Ricevi" (how others see this phone, what arrived). Ping is the bell at
 * the top, always at hand.
 */
@Composable
fun SendScreen(current: JSONObject?, open: (Route) -> Unit) {
    val receive by SendNav.receive.collectAsState()
    LaunchedEffect(Unit) { refreshLocalSend() }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(top = 6.dp, bottom = 8.dp)) {
            SingleChoiceSegmentedButtonRow(Modifier.weight(1f)) {
                SegmentedButton(
                    selected = !receive, onClick = { SendNav.receive.value = false },
                    shape = SegmentedButtonDefaults.itemShape(0, 2),
                    icon = { Icon(Icons.AutoMirrored.Filled.Send, null, Modifier.size(18.dp)) },
                ) { Text("Invia") }
                SegmentedButton(
                    selected = receive, onClick = { SendNav.receive.value = true },
                    shape = SegmentedButtonDefaults.itemShape(1, 2),
                    icon = { Icon(Icons.Filled.Download, null, Modifier.size(18.dp)) },
                ) { Text("Ricevi") }
            }
            Spacer(Modifier.width(10.dp))
            PingButton(current)
        }
        if (receive) ReceivePane() else SendPane(current, open)
        Spacer(Modifier.height(16.dp))
    }
}

@Composable
private fun PingButton(current: JSONObject?) {
    val ui = LocalUi.current
    val scope = rememberCoroutineScope()
    val blocked = blockedReason(current, "ping", Types.PING)
    FilledTonalIconButton(onClick = {
        if (blocked != null) ui.toast(blocked)
        else scope.launch {
            val id = current!!.optString("id")
            core(ui.toast) { it.sendPacket(id, Types.PING, JSONObject().put("message", "Ping dal telefono")) }
                ?.let { ui.toast("Ping inviato a ${current.optString("name")}") }
        }
    }) {
        Icon(Icons.Filled.NotificationsActive, "Ping al computer",
            tint = if (blocked == null) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = .5f))
    }
}

/** A device to send to: a paired computer or a LocalSend device. */
private class Target(
    val icon: ImageVector,
    val name: String,
    val tags: List<Pair<String, Color?>>,
    val blocked: String?,
    val send: (List<Pick>) -> Unit,
)

@Composable
private fun SendPane(current: JSONObject?, open: (Route) -> Unit) {
    val ctx = LocalContext.current
    val ui = LocalUi.current
    val scope = rememberCoroutineScope()
    val sel by SendNav.selection.collectAsState()
    val devices by Store.devices.collectAsState()
    val status by Store.lsStatus.collectAsState()
    val peers by Store.lsPeers.collectAsState()
    var scanning by remember { mutableStateOf(false) }
    var askText by remember { mutableStateOf(false) }
    var askAddress by remember { mutableStateOf(false) }
    val add = { l: List<Pick> -> SendNav.selection.update { it + l } }
    val pickFiles = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        add(uris.map { pickOf(ctx, it) })
    }
    val pickMedia = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia()) { uris ->
        add(uris.map { pickOf(ctx, it) })
    }
    val pickers = listOf(
        Triple(Icons.Filled.Description, "File") { pickFiles.launch(arrayOf("*/*")) },
        Triple(Icons.Filled.Image, "Media") {
            pickMedia.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo))
        },
        Triple(Icons.Filled.TextFields, "Testo") { askText = true },
        Triple(Icons.Filled.ContentPaste, "Appunti") {
            val t = ui.clipboard()
            if (t.isNullOrEmpty()) ui.toast("Gli appunti sono vuoti")
            else add(listOf(Pick("clipboard", name = "Appunti", size = t.length.toLong(), text = t)))
        },
    )

    if (sel.isEmpty()) {
        SectionTitle("Selezione")
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            pickers.forEach { (icon, label, go) -> BigButton(icon, label, Modifier.weight(1f), go) }
        }
    } else {
        SelectionCard(sel, pickers)
    }

    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(top = 12.dp)) {
        Text("Dispositivi vicini", Modifier.weight(1f), style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant)
        val spin by rememberInfiniteTransition(label = "scan").animateFloat(
            0f, 360f, infiniteRepeatable(tween(1000, easing = LinearEasing)), label = "scan")
        IconButton(onClick = {
            scope.launch {
                core(ui.toast) { it.callArray("discovered") }?.let { Store.nearby.value = it.objects() }
                if (status?.optBoolean("running") == true) {
                    scanning = true
                    core(ui.toast) { it.pluginAction("localsend", "scan") }
                    delay(6000)
                    scanning = false
                }
            }
        }) { Icon(Icons.Filled.Sync, "Cerca di nuovo", Modifier.rotate(if (scanning) spin else 0f), tint = MaterialTheme.colorScheme.primary) }
        if (status?.optBoolean("running") == true) {
            IconButton(onClick = { askAddress = true }) {
                Icon(Icons.Filled.AdsClick, "Invia a un indirizzo", tint = MaterialTheme.colorScheme.primary)
            }
        }
    }

    val s = LocalStatus.current
    val scheme = MaterialTheme.colorScheme
    val targets = devices.filter { it.optBoolean("paired") }.map { d ->
        val name = d.optString("name")
        Target(
            icon = when (d.optString("deviceType")) {
                "phone" -> Icons.Filled.Smartphone
                "tablet" -> Icons.Filled.Tablet
                "laptop" -> Icons.Filled.Laptop
                else -> Icons.Filled.Computer
            },
            name = name,
            tags = listOfNotNull(
                "Konnekta" to scheme.primary,
                d.optString("transport").takeIf { d.optBoolean("connected") && it.isNotEmpty() }
                    ?.let { (mapOf("wifi" to "Wi-Fi", "usb" to "Cavo", "bluetooth" to "Bluetooth")[it] ?: it) to null },
                if (d.optBoolean("connected")) null else "Non raggiungibile" to s.muted,
            ),
            blocked = blockedReason(d, "files", Types.SHARE),
        ) { picks -> sendToComputer(ctx, ui, d.optString("id"), name, picks) }
    } + (if (status?.optBoolean("enabled") == true) peers else emptyList()).map { p ->
        val name = p.optString("alias")
        Target(
            icon = when (p.optString("deviceType")) {
                "mobile" -> Icons.Filled.Smartphone
                "web" -> Icons.Filled.Language
                else -> Icons.Filled.Computer
            },
            name = name,
            tags = listOfNotNull(
                "LocalSend" to s.ok,
                "#${p.optString("ip").substringAfterLast('.')}" to null,
                p.optString("deviceModel").takeIf { it.isNotEmpty() }?.let { it to null },
            ),
            blocked = null,
        ) { picks -> sendToLocalSend(ctx, ui, p.optString("id"), name, picks, null) }
    }

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if (targets.isEmpty()) {
            DeviceTile(Icons.Filled.Devices, "Nessun dispositivo vicino",
                listOf("Apri Konnekta o LocalSend sull'altro dispositivo, nella stessa rete" to null), dim = true, onClick = null)
        }
        targets.forEach { t ->
            DeviceTile(t.icon, t.name, t.tags, dim = t.blocked != null) {
                when {
                    t.blocked != null -> ui.toast(t.blocked)
                    sel.isEmpty() -> ui.toast("Scegli prima cosa inviare: file, media, testo o appunti")
                    else -> t.send(sel)
                }
            }
        }
    }
    when {
        status == null -> {}
        status?.optBoolean("enabled") != true -> LocalSendOffCard()
        status?.optString("error")?.takeIf { it.isNotEmpty() && it != "null" } != null ->
            Hint(status!!.optString("error"), Modifier.padding(top = 4.dp))
    }

    InProgress(incoming = false)

    SectionTitle("Altro")
    FnList(listOf(
        Fn(Icons.Filled.FolderOpen, "Prendi file dal computer", "Sfoglia le cartelle condivise del computer",
            blocked = blockedReason(current, "browseFiles", Types.BROWSE_REQUEST)) { open(Route.Browse) },
    ))
    Hint("Scegli cosa inviare, poi tocca il dispositivo. I computer con Konnekta ricevono subito, i dispositivi con LocalSend chiedono conferma. " +
        "Da un'altra app: Condividi › Konnekta.")

    if (askText) TextDialog(onDismiss = { askText = false }) { text ->
        add(listOf(Pick("text", name = text.take(40), size = text.length.toLong(), text = text)))
        askText = false
    }
    if (askAddress) AddressDialog(onDismiss = { askAddress = false })
}

/** One of the LocalSend selection buttons: big icon, label below. */
@Composable
private fun BigButton(icon: ImageVector, label: String, modifier: Modifier, onClick: () -> Unit) {
    Surface(
        onClick = onClick,
        shape = RoundedCornerShape(16.dp),
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        border = BorderStroke(1.dp, LocalStatus.current.border),
        modifier = modifier,
    ) {
        Column(Modifier.padding(vertical = 16.dp).fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Icon(icon, null, Modifier.size(28.dp), tint = MaterialTheme.colorScheme.primary)
            Text(label, style = MaterialTheme.typography.labelLarge, maxLines = 1)
        }
    }
}

@Composable
private fun SelectionCard(sel: List<Pick>, pickers: List<Triple<ImageVector, String, () -> Unit>>) {
    var menu by remember { mutableStateOf(false) }
    Surface(
        shape = RoundedCornerShape(16.dp),
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        border = BorderStroke(1.dp, LocalStatus.current.border),
        modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
    ) {
        Column(Modifier.padding(14.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text("Selezione", style = MaterialTheme.typography.titleMedium)
                    val files = sel.count { it.kind == "file" }
                    Text(
                        listOfNotNull(
                            if (files > 0) (if (files == 1) "1 file" else "$files file") else null,
                            (sel.size - files).takeIf { it > 0 }?.let { if (it == 1) "1 testo" else "$it testi" },
                            humanSize(sel.filter { it.kind == "file" }.sumOf { it.size }).takeIf { files > 0 },
                        ).joinToString(" · "),
                        style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                IconButton(onClick = { SendNav.selection.value = emptyList() }) { Icon(Icons.Filled.Close, "Svuota") }
            }
            LazyRow(Modifier.padding(vertical = 10.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                items(sel.size) { i -> Thumb(sel[i]) { SendNav.selection.update { l -> l.filterIndexed { j, _ -> j != i } } } }
            }
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                Box {
                    Button(onClick = { menu = true }) {
                        Icon(Icons.Filled.Add, null, Modifier.size(18.dp))
                        Spacer(Modifier.width(6.dp))
                        Text("Aggiungi")
                    }
                    DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                        pickers.forEach { (icon, label, go) ->
                            DropdownMenuItem(text = { Text(label) }, leadingIcon = { Icon(icon, null) }, onClick = { menu = false; go() })
                        }
                    }
                }
            }
        }
    }
}

private fun pickIcon(p: Pick): ImageVector = when {
    p.kind == "clipboard" -> Icons.Filled.ContentPaste
    p.kind == "text" -> Icons.Filled.TextFields
    p.mime.startsWith("image/") -> Icons.Filled.Image
    p.mime.startsWith("video/") -> Icons.Filled.Movie
    p.mime.startsWith("audio/") -> Icons.Filled.MusicNote
    else -> Icons.Filled.Description
}

@Composable
private fun Thumb(p: Pick, onRemove: () -> Unit) {
    Surface(
        shape = RoundedCornerShape(12.dp),
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        modifier = Modifier.width(92.dp),
    ) {
        Box {
            Column(Modifier.padding(start = 8.dp, end = 8.dp, top = 12.dp, bottom = 8.dp).fillMaxWidth(),
                horizontalAlignment = Alignment.CenterHorizontally) {
                Icon(pickIcon(p), null, Modifier.size(30.dp), tint = MaterialTheme.colorScheme.primary)
                Spacer(Modifier.height(4.dp))
                Text(p.name, style = MaterialTheme.typography.labelSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            IconButton(onClick = onRemove, modifier = Modifier.align(Alignment.TopEnd).size(28.dp)) {
                Icon(Icons.Filled.Close, "Togli", Modifier.size(14.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

/** A device as LocalSend shows it: round icon, name, small tags. */
@Composable
private fun DeviceTile(icon: ImageVector, name: String, tags: List<Pair<String, Color?>>, dim: Boolean = false, onClick: (() -> Unit)?) {
    val scheme = MaterialTheme.colorScheme
    Surface(
        onClick = onClick ?: {},
        enabled = onClick != null,
        shape = RoundedCornerShape(16.dp),
        color = scheme.surfaceContainerLow,
        border = BorderStroke(1.dp, LocalStatus.current.border),
        modifier = Modifier.fillMaxWidth().alpha(if (dim) .55f else 1f),
    ) {
        Row(Modifier.padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(48.dp).clip(CircleShape).background(scheme.primaryContainer), contentAlignment = Alignment.Center) {
                Icon(icon, null, tint = scheme.onPrimaryContainer)
            }
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(name, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    tags.forEach { (t, c) -> Pill(t, c ?: scheme.onSurfaceVariant) }
                }
            }
        }
    }
}

@Composable
private fun LocalSendOffCard() {
    val ui = LocalUi.current
    val scope = rememberCoroutineScope()
    Surface(
        shape = RoundedCornerShape(16.dp),
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        border = BorderStroke(1.dp, LocalStatus.current.border),
        modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
    ) {
        Row(Modifier.padding(14.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text("Compatibilità LocalSend", style = MaterialTheme.typography.titleSmall)
                Text("Attivala per vedere anche i telefoni e i computer con l'app LocalSend e scambiare file con loro.",
                    style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Spacer(Modifier.width(8.dp))
            Button(onClick = { scope.launch { setLocalSend(ui, JSONObject().put("enabled", true)) } }) { Text("Attiva") }
        }
    }
}

/** Change LocalSend settings in the core; the status comes back with the answer. */
suspend fun setLocalSend(ui: Ui, changes: JSONObject) {
    core(ui.toast) { it.pluginAction("localsend", "setConfig", changes) as? JSONObject }?.let { Store.lsStatus.value = it }
}

/** Transfers running now (both directions, Konnekta and LocalSend). */
@Composable
private fun InProgress(incoming: Boolean) {
    val ui = LocalUi.current
    val scope = rememberCoroutineScope()
    val ls by Store.lsTransfers.collectAsState()
    val konnekta by Store.transfers.collectAsState()
    val running = ls.values.filter {
        it.optString("state") in setOf("waiting", "inProgress") && (it.optString("direction") == "incoming") == incoming
    }
    if (running.isEmpty() && konnekta.isEmpty()) return
    SectionTitle("In corso")
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        konnekta.values.forEach { label -> SmallCard("Konnekta", MaterialTheme.colorScheme.primary, label, "") }
        running.forEach { t ->
            val total = t.optLong("total")
            val done = t.optLong("transferred")
            val files = t.optJSONArray("files")
            val what = if (files?.length() == 1) files.optJSONObject(0)?.optString("name").orEmpty() else "${files?.length() ?: 0} file"
            Surface(
                shape = RoundedCornerShape(14.dp),
                color = MaterialTheme.colorScheme.surfaceContainerLow,
                border = BorderStroke(1.dp, LocalStatus.current.border),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Column(Modifier.padding(start = 14.dp, end = 4.dp, top = 10.dp, bottom = 10.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text(what, style = MaterialTheme.typography.titleSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            Text(
                                (if (incoming) "Da " else "A ") + t.optString("peer") + " · " +
                                    if (t.optString("state") == "waiting") "in attesa che accetti" else "${humanSize(done)} di ${humanSize(total)}",
                                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        TextButton(onClick = {
                            scope.launch { core(ui.toast) { it.pluginAction("localsend", "cancel", JSONObject().put("id", t.optString("id"))) } }
                        }) { Text("Annulla") }
                    }
                    LinearProgressIndicator(
                        progress = { if (total > 0) done.toFloat() / total else 0f },
                        modifier = Modifier.fillMaxWidth().padding(end = 10.dp, top = 6.dp),
                    )
                }
            }
        }
    }
}

/** How this phone appears to the others, and what it received. */
@Composable
private fun ReceivePane() {
    val ui = LocalUi.current
    val scope = rememberCoroutineScope()
    val status by Store.lsStatus.collectAsState()
    val ctx = LocalContext.current
    val st = status
    val name = st?.optString("alias")?.takeIf { it.isNotEmpty() }
        ?: ctx.getSharedPreferences("konnekta", android.content.Context.MODE_PRIVATE).getString("deviceName", null)
        ?: android.os.Build.MODEL
    val pulse by rememberInfiniteTransition(label = "ready").animateFloat(
        0f, 1f, infiniteRepeatable(tween(2400, easing = LinearEasing)), label = "ready")
    val scheme = MaterialTheme.colorScheme
    Column(Modifier.fillMaxWidth().padding(top = 12.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Box(Modifier.size(190.dp), contentAlignment = Alignment.Center) {
            // A ring that grows and fades: ready to receive.
            Box(Modifier.size(150.dp).scale(1f + pulse * .25f).alpha((1f - pulse) * .7f).clip(CircleShape)
                .border(3.dp, scheme.primary, CircleShape))
            Box(Modifier.size(150.dp).clip(CircleShape).background(Color(0xFF1E1E2E)), contentAlignment = Alignment.Center) {
                Icon(painterResource(org.konnekta.app.R.drawable.ic_launcher_fg), null, Modifier.size(150.dp), tint = Color.Unspecified)
            }
        }
        Text(name, style = MaterialTheme.typography.headlineMedium, modifier = Modifier.padding(top = 8.dp))
        val ips = st?.optJSONArray("ips")?.let { a -> (0 until a.length()).map { a.optString(it) } }.orEmpty()
        if (ips.isNotEmpty()) Row(Modifier.padding(top = 6.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            ips.forEach { Pill("#${it.substringAfterLast('.')}", scheme.onSurfaceVariant) }
        }
        Column(Modifier.padding(vertical = 14.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                StatusDot(true)
                Spacer(Modifier.width(8.dp))
                Text("Konnekta: i computer associati inviano senza chiedere", style = MaterialTheme.typography.bodySmall)
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                StatusDot(st?.optBoolean("running") == true)
                Spacer(Modifier.width(8.dp))
                Text(
                    when {
                        st == null -> "LocalSend: non disponibile"
                        !st.optBoolean("enabled") -> "LocalSend: disattivato"
                        st.optBoolean("running") -> "LocalSend: visibile come «${st.optString("alias")}»"
                        else -> "LocalSend: ${st.optString("error").takeIf { it.isNotEmpty() && it != "null" } ?: "avvio…"}"
                    },
                    Modifier.weight(1f, fill = false), style = MaterialTheme.typography.bodySmall,
                )
            }
        }
        if (st != null && !st.optBoolean("enabled")) {
            Button(onClick = { scope.launch { setLocalSend(ui, JSONObject().put("enabled", true)) } }) { Text("Attiva LocalSend") }
        }
        if (st?.optBoolean("enabled") == true) {
            Text("Ricezione rapida (LocalSend)", style = MaterialTheme.typography.labelMedium, color = scheme.onSurfaceVariant)
            val quick = st.optBoolean("autoAccept")
            SingleChoiceSegmentedButtonRow(Modifier.padding(top = 6.dp)) {
                SegmentedButton(selected = !quick, onClick = { scope.launch { setLocalSend(ui, JSONObject().put("autoAccept", false)) } },
                    shape = SegmentedButtonDefaults.itemShape(0, 2)) { Text("Chiedi sempre") }
                SegmentedButton(selected = quick, onClick = { scope.launch { setLocalSend(ui, JSONObject().put("autoAccept", true)) } },
                    shape = SegmentedButtonDefaults.itemShape(1, 2)) { Text("Accetta tutto") }
            }
        }
    }
    InProgress(incoming = true)
    Received()
}

/** What arrived: transfers in progress, files (tap to open), links, texts. */
@Composable
private fun Received() {
    val items by Store.inbox.collectAsState()
    val shown = items.filter { it.kind !in setOf("photo", "ring") }
    Row(verticalAlignment = Alignment.CenterVertically) {
        SectionTitle("Ricevuti")
        Spacer(Modifier.weight(1f))
        if (shown.isNotEmpty()) TextButton(onClick = { Store.inbox.value = items.filter { it.kind in setOf("photo", "ring") } }) {
            Text("Svuota")
        }
    }
    if (shown.isEmpty()) {
        Hint("I file che ricevi compaiono qui: toccali per aprirli. Li trovi anche in Download/${Files.FOLDER}.")
        return
    }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) { shown.forEach { InboxCard(it) } }
}

@Composable
private fun TextDialog(onDismiss: () -> Unit, onDone: (String) -> Unit) {
    var text by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Testo da inviare") },
        text = { OutlinedTextField(text, { text = it }, Modifier.fillMaxWidth().heightIn(min = 120.dp), placeholder = { Text("Testo o link") }) },
        confirmButton = { Button(onClick = { if (text.isNotBlank()) onDone(text) }, enabled = text.isNotBlank()) { Text("Aggiungi") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Annulla") } },
    )
}

@Composable
private fun AddressDialog(onDismiss: () -> Unit) {
    val ui = LocalUi.current
    val scope = rememberCoroutineScope()
    var address by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Invia a un indirizzo") },
        text = {
            Column {
                Text("L'indirizzo IP di un dispositivo con LocalSend (lo trovi nella sua pagina Ricevi).",
                    style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                OutlinedTextField(address, { address = it }, Modifier.fillMaxWidth().padding(top = 8.dp),
                    placeholder = { Text("192.168.1.20") }, singleLine = true)
            }
        },
        confirmButton = {
            Button(onClick = {
                scope.launch {
                    core(ui.toast) { it.pluginAction("localsend", "addPeer", JSONObject().put("address", address.trim())) }?.let {
                        ui.toast("Dispositivo trovato")
                        refreshLocalSend()
                        onDismiss()
                    }
                }
            }, enabled = address.isNotBlank()) { Text("Cerca") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Annulla") } },
    )
}

/** Files, texts and the clipboard to a paired computer, the Konnekta way. */
@OptIn(kotlinx.coroutines.DelicateCoroutinesApi::class)
private fun sendToComputer(ctx: android.content.Context, ui: Ui, deviceId: String, name: String, picks: List<Pick>) {
    picks.filter { it.kind != "file" }.forEach { p ->
        val text = p.text.orEmpty()
        GlobalScope.launch(Dispatchers.Main) {
            val url = text.trim().takeIf { Regex("^https?://\\S+$").matches(it) }
            val ok = core(ui.toast) {
                when {
                    p.kind == "clipboard" -> it.sendPacket(deviceId, Types.CLIPBOARD, JSONObject().put("content", text))
                    url != null -> it.sendPacket(deviceId, Types.SHARE, JSONObject().put("url", url))
                    else -> it.sendPacket(deviceId, Types.SHARE, JSONObject().put("text", text))
                }
            } != null
            if (ok) ui.toast(if (p.kind == "clipboard") "Appunti inviati a $name" else "Testo inviato a $name")
        }
    }
    val uris = picks.filter { it.kind == "file" }.mapNotNull { it.uri }
    if (uris.isNotEmpty()) sendUris(ctx, ui, deviceId, uris)
    SendNav.selection.value = emptyList()
}

/**
 * The same to a LocalSend device: texts as messages, then the files
 * (copied to the cache so the core can read them). The device asks its
 * user first, so this waits up to a few minutes; it runs in the app scope.
 */
@OptIn(kotlinx.coroutines.DelicateCoroutinesApi::class)
private fun sendToLocalSend(ctx: android.content.Context, ui: Ui, peerId: String, name: String, picks: List<Pick>, pin: String?) {
    val app = ctx.applicationContext
    GlobalScope.launch(Dispatchers.Main) {
        val label = "LocalSend: invio a $name"
        val t = Store.startTransfer(label)
        val ok = core({ msg ->
            if (msg.contains("PIN")) LocalSendPin.ask.value = Triple(peerId, name, picks)
            ui.toast(msg)
        }) { c ->
            picks.filter { it.kind != "file" }.forEach { p ->
                c.pluginAction("localsend", "send", JSONObject().put("peerId", peerId).put("text", p.text.orEmpty()).put("pin", pin ?: ""))
            }
            val staged = picks.filter { it.kind == "file" }.mapNotNull { it.uri }.map { Files.stage(app, it).first }
            try {
                if (staged.isNotEmpty()) {
                    c.pluginAction("localsend", "send", JSONObject().put("peerId", peerId).put("pin", pin ?: "")
                        .put("paths", JSONArray(staged.map { it.absolutePath })))
                }
            } finally {
                staged.forEach { it.parentFile?.deleteRecursively() }
            }
        } != null
        Store.endTransfer(t)
        if (ok) {
            ui.toast("Inviato a $name")
            SendNav.selection.value = emptyList()
        }
    }
}

/** A LocalSend device asked for a PIN: the dialog that retries with it. */
object LocalSendPin {
    val ask = MutableStateFlow<Triple<String, String, List<Pick>>?>(null)
}

@Composable
fun LocalSendPinDialog() {
    val ctx = LocalContext.current
    val ui = LocalUi.current
    val a by LocalSendPin.ask.collectAsState()
    val (peerId, name, picks) = a ?: return
    var pin by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = { LocalSendPin.ask.value = null },
        title = { Text("PIN per $name") },
        text = {
            OutlinedTextField(pin, { pin = it }, Modifier.fillMaxWidth(), singleLine = true, placeholder = { Text("PIN") },
                keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(keyboardType = androidx.compose.ui.text.input.KeyboardType.Number))
        },
        confirmButton = {
            Button(onClick = {
                LocalSendPin.ask.value = null
                sendToLocalSend(ctx, ui, peerId, name, picks, pin)
            }, enabled = pin.isNotBlank()) { Text("Invia") }
        },
        dismissButton = { TextButton(onClick = { LocalSendPin.ask.value = null }) { Text("Annulla") } },
    )
}

/** Transfers outlive the screen: they run in the app scope. */
@OptIn(kotlinx.coroutines.DelicateCoroutinesApi::class)
fun sendUris(ctx: android.content.Context, ui: Ui, deviceId: String, uris: List<android.net.Uri>) {
    val app = ctx.applicationContext
    GlobalScope.launch(Dispatchers.Main) {
        val label = if (uris.size == 1) "Invio di 1 file" else "Invio di ${uris.size} file"
        val t = Store.startTransfer(label)
        val ok = core(ui.toast) { Files.send(app, it, deviceId, uris) } != null
        Store.endTransfer(t)
        if (ok) ui.toast(if (uris.size == 1) "File inviato" else "${uris.size} file inviati")
    }
}

/** Shared folders of the computer: roots, folders, search, download. */
@Composable
fun BrowseScreen(current: JSONObject?, onBack: () -> Unit) {
    val ui = LocalUi.current
    val scope = rememberCoroutineScope()
    val id = current?.optString("id") ?: return onBack()
    val roots by Store.browseRoots.collectAsState()
    val answers by Store.browse.collectAsState()
    var root by remember { mutableStateOf("home") }
    var path by remember { mutableStateOf("") }
    var query by remember { mutableStateOf("") }
    fun req(body: JSONObject) = scope.launch { send(ui.toast, id, Types.BROWSE_REQUEST, body) }
    fun list() = req(JSONObject().put("action", "list").put("root", root).put("path", path))

    LaunchedEffect(id) {
        Store.put(Store.browse, id, JSONObject())
        req(JSONObject().put("action", "roots"))
    }
    LaunchedEffect(root, path) { list() }
    DisposableEffect(id) { onDispose { CoroutineScope(Dispatchers.IO).launch { send({}, id, Types.BROWSE_REQUEST, JSONObject().put("action", "stop")) } } }

    val answer = answers[id]
    val entries = answer?.optJSONArray("entries")?.objects().orEmpty()
    val rootList = roots[id]?.objects().orEmpty()
    BackHandlerFor(path.isNotEmpty()) { path = path.substringBeforeLast('/', "") }

    SubHeader("Prendi file", if (path.isEmpty()) rootList.firstOrNull { it.optString("id") == root }?.optString("label") ?: current.optString("name") else path, onBack)
    OutlinedTextField(
        query, { query = it }, Modifier.fillMaxWidth(),
        placeholder = { Text("Cerca nelle cartelle condivise") },
        leadingIcon = { Icon(Icons.Filled.Search, null) },
        singleLine = true,
        keyboardActions = androidx.compose.foundation.text.KeyboardActions(onSearch = {
            if (query.isBlank()) list() else req(JSONObject().put("action", "search").put("root", root).put("path", path).put("query", query.trim()))
        }),
        keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(imeAction = androidx.compose.ui.text.input.ImeAction.Search),
    )
    if (rootList.size > 1) {
        LazyRow(Modifier.padding(vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            items(rootList) { r ->
                FilterChip(selected = r.optString("id") == root, onClick = {
                    root = r.optString("id")
                    path = ""
                    query = ""
                }, label = { Text(r.optString("label")) })
            }
        }
    }
    answer?.takeIf { it.optString("action") == "error" }?.let { Hint(it.optString("error")) }
    LazyColumn(Modifier.fillMaxSize()) {
        items(entries, key = { it.optString("path") + it.optString("name") }) { e ->
            val dir = e.optBoolean("dir")
            ActionCard(
                if (dir) Icons.Filled.Folder else Icons.AutoMirrored.Filled.InsertDriveFile,
                e.optString("name"),
                if (dir) "Cartella" else humanSize(e.optLong("size")),
                trailing = { Icon(if (dir) Icons.AutoMirrored.Filled.KeyboardArrowRight else Icons.Filled.Download, null,
                    tint = MaterialTheme.colorScheme.primary) },
            ) {
                val p = e.optString("path")
                if (dir) {
                    query = ""
                    path = p
                } else {
                    req(JSONObject().put("action", "get").put("root", root).put("path", p))
                    ui.toast("Scarico ${e.optString("name")} in Download/${Files.FOLDER}")
                }
            }
        }
        if (entries.isEmpty() && answer?.has("entries") == true) item { Hint("Cartella vuota.") }
    }
}

@Composable
fun BackHandlerFor(enabled: Boolean, onBack: () -> Unit) = androidx.activity.compose.BackHandler(enabled, onBack)

fun humanSize(n: Long): String = when {
    n < 1024 -> "$n B"
    n < 1024 * 1024 -> "${n / 1024} KB"
    n < 1024L * 1024 * 1024 -> "%.1f MB".format(n / 1048576.0)
    else -> "%.1f GB".format(n / 1073741824.0)
}
