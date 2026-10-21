package org.konnekta.app.ui

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
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.filled.GridView
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.LinkOff
import androidx.compose.material.icons.filled.Notifications
import androidx.compose.material.icons.filled.PhoneAndroid
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.json.JSONObject
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material.icons.filled.Call
import androidx.compose.material.icons.filled.DoNotDisturbOn
import androidx.compose.material.icons.filled.Mic
import androidx.compose.material.icons.filled.MusicNote
import androidx.compose.material.icons.filled.Videocam
import androidx.compose.material.icons.filled.Screenshot
import androidx.compose.material.icons.filled.Sms
import org.konnekta.app.service.Calls
import org.konnekta.app.service.Dnd
import org.konnekta.app.service.KonnektaService
import org.konnekta.app.service.Screenshots
import org.konnekta.app.service.Sms
import org.konnekta.app.service.NotificationForwarder
import org.konnekta.app.state.Store

@Composable
fun ComputersScreen(devices: List<JSONObject>, open: (Route) -> Unit) {
    val ctx = LocalContext.current
    val ui = LocalUi.current
    val scope = rememberCoroutineScope()
    val nearby by Store.nearby.collectAsState()
    var address by remember { mutableStateOf("") }
    var me by remember { mutableStateOf<JSONObject?>(null) }
    LaunchedEffect(Unit) { me = core(ui.toast) { it.callObject("info") } }
    val paired = devices.filter { it.optBoolean("paired") }
    val available = availableComputers(devices, nearby)

    fun pair(d: JSONObject) = pairWith(scope, ui, d)

    LazyColumn(Modifier.fillMaxSize()) {
        item {
            Row(Modifier.padding(vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Filled.PhoneAndroid, null, tint = MaterialTheme.colorScheme.primary)
                Spacer(Modifier.width(12.dp))
                Column {
                    Text(me?.optString("deviceName") ?: "Questo telefono", style = MaterialTheme.typography.titleMedium)
                    Text("Visibile ai computer su questa rete", style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        }
        item { SectionTitle("Associati") }
        if (paired.isEmpty()) item { Hint("Nessun computer associato: scegline uno qui sotto.") }
        items(paired, key = { "p" + it.optString("id") }) { d ->
            val online = d.optBoolean("connected")
            ActionCard(
                if (d.optString("deviceType") in setOf("phone", "tablet")) Icons.Filled.PhoneAndroid else Icons.Filled.Computer, d.optString("name"),
                if (online) "Connesso · ${d.optString("address")}"
                else "Non raggiungibile${d.optString("address").takeIf { it.isNotEmpty() }?.let { " · $it" } ?: ""}",
                trailing = {
                    if (online) StatusDot(true) else TextButton(onClick = {
                        scope.launch {
                            d.optString("address").takeIf { it.isNotEmpty() }?.let { a ->
                                core(ui.toast) { it.call("connect", JSONObject().put("address", a)) }
                            }
                        }
                    }) { Text("Riprova") }
                },
            ) { open(Route.Device(d.optString("id"))) }
        }
        item { SectionTitle("Disponibili") }
        if (available.isEmpty()) item { Hint("Avvia Konnekta sul computer, nella stessa rete Wi-Fi.") }
        items(available, key = { "n" + it.optString("id") }) { d -> AvailableCard(d) { pair(d) } }
        item {
            Row(Modifier.padding(top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                OutlinedTextField(address, { address = it }, Modifier.weight(1f), label = { Text("Indirizzo (anche Tailscale)") }, singleLine = true)
                Spacer(Modifier.width(8.dp))
                OutlinedButton(enabled = address.isNotBlank(), onClick = {
                    scope.launch {
                        core(ui.toast) { it.call("connect", JSONObject().put("address", address.trim())) }?.let { ui.toast("Richiesta inviata") }
                    }
                }) { Text("Connetti") }
            }
        }
        item {
            TextButton(onClick = { scope.launch { core(ui.toast) { it.call("refresh") } } }) {
                Icon(Icons.Filled.Refresh, null)
                Spacer(Modifier.width(6.dp))
                Text("Cerca di nuovo")
            }
        }
        item { SectionTitle("Impostazioni") }
        item { LayoutSetting() }
        item { NotificationsSetting() }
        item { PhoneFeatures() }
        item {
            ActionCard(Icons.Filled.Info, "Informazioni", me?.let { "Konnekta ${it.optString("version", "0.1")} · ${it.optString("deviceId").take(8)}" } ?: "Konnekta") {}
        }
    }
}

/** Dial the computer if needed, ask to pair and show our key while it decides. */
fun pairWith(scope: kotlinx.coroutines.CoroutineScope, ui: Ui, d: JSONObject) = scope.launch {
    val id = d.optString("id")
    if (!d.optBoolean("connected") && d.has("address")) {
        core(ui.toast) { it.call("connect", JSONObject().put("address", d.optString("address"))) }
        delay(1500)
    }
    val r = core(ui.toast) { it.callObject("pair", JSONObject().put("deviceId", id)) }
    r?.optString("verificationKey")?.takeIf { it.isNotEmpty() }?.let { key ->
        KonnektaService.pairPrompt.value = JSONObject().put("deviceId", id).put("deviceName", d.optString("name"))
            .put("verificationKey", key).put("waiting", true)
    }
}

/** Computers on the network that are not paired yet (phones excluded). */
fun availableComputers(devices: List<JSONObject>, nearby: List<JSONObject>): List<JSONObject> {
    val paired = devices.filter { it.optBoolean("paired") }
    return (devices.filter { !it.optBoolean("paired") } + nearby).distinctBy { it.optString("id") }
        .filter { d -> paired.none { it.optString("id") == d.optString("id") } }
        .filter { it.optString("deviceType") !in setOf("phone", "tablet") }
}

@Composable
fun AvailableCard(d: JSONObject, onClick: () -> Unit) {
    val via = d.optString("protocol").takeIf { it.isNotEmpty() && !it.equals("konnekta", true) }
    Surface(
        onClick = onClick,
        shape = RoundedCornerShape(14.dp),
        color = MaterialTheme.colorScheme.surface,
        border = BorderStroke(1.dp, LocalStatus.current.warn.copy(alpha = .7f)),
        modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp),
    ) {
        Row(Modifier.padding(14.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Filled.Add, null, tint = LocalStatus.current.warn)
            Spacer(Modifier.width(14.dp))
            Column {
                Text(d.optString("name"), style = MaterialTheme.typography.titleMedium)
                Text(if (via != null) "Tocca per associare · tramite $via" else "Tocca per associare",
                    style = MaterialTheme.typography.bodySmall, color = LocalStatus.current.warn)
            }
        }
    }
}

/** Features one per line, or two per row. */
@Composable
private fun LayoutSetting() {
    val ctx = LocalContext.current
    val grid by Store.grid.collectAsState()
    ActionCard(
        Icons.Filled.GridView, "Funzioni in griglia",
        if (grid) "Due per riga, più compatte" else "Una per riga, con la descrizione completa",
        trailing = { Switch(grid, { Store.setGrid(ctx, it) }) },
    ) { Store.setGrid(ctx, !grid) }
}

@Composable
fun NotificationsSetting() {
    val ctx = LocalContext.current
    var on by remember { mutableStateOf(NotificationForwarder.enabled(ctx)) }
    // Re-check when coming back from the system settings.
    val owner = LocalLifecycleOwner.current
    DisposableEffect(owner) {
        val obs = LifecycleEventObserver { _, e -> if (e == Lifecycle.Event.ON_RESUME) on = NotificationForwarder.enabled(ctx) }
        owner.lifecycle.addObserver(obs)
        onDispose { owner.lifecycle.removeObserver(obs) }
    }
    ActionCard(
        Icons.Filled.Notifications, "Inoltra le notifiche",
        if (on) "Attivo: i computer con il permesso Notifiche le ricevono" else "Mostra sul computer le notifiche del telefono",
        trailing = { Switch(on, { ctx.startActivity(NotificationForwarder.settingsIntent()) }) },
    ) { ctx.startActivity(NotificationForwarder.settingsIntent()) }
}

/**
 * Android permissions behind each phone feature. The computer still needs
 * its own Konnekta permission (Calls, SMS…) on top of these.
 */
@Composable
fun PhoneFeatures() {
    val ctx = LocalContext.current
    var tick by remember { mutableStateOf(0) }
    val owner = LocalLifecycleOwner.current
    DisposableEffect(owner) {
        val obs = LifecycleEventObserver { _, e -> if (e == Lifecycle.Event.ON_RESUME) tick++ }
        owner.lifecycle.addObserver(obs)
        onDispose { owner.lifecycle.removeObserver(obs) }
    }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
        tick++
        KonnektaService.refreshFeatures(ctx)
    }
    fun has(vararg p: String) = p.all { ctx.checkSelfPermission(it) == android.content.pm.PackageManager.PERMISSION_GRANTED }
    @Suppress("UNUSED_EXPRESSION") tick
    val listener = NotificationForwarder.enabled(ctx)
    Column {
        Feature(Icons.Filled.MusicNote, "Media del telefono", "Controlla dal computer la musica del telefono (usa l'inoltro notifiche)", listener) {
            ctx.startActivity(NotificationForwarder.settingsIntent())
        }
        Feature(Icons.Filled.Call, "Chiamate", "Chi chiama, rispondi o rifiuta dal computer · sensibile", has(*Calls.PERMISSIONS)) { ask.launch(Calls.PERMISSIONS) }
        Feature(Icons.Filled.Sms, "SMS", "Leggi e invia messaggi dal computer · sensibile", has(*Sms.PERMISSIONS)) { ask.launch(Sms.PERMISSIONS) }
        Feature(Icons.Filled.Screenshot, "Screenshot sul computer", "Ogni screenshot del telefono arriva sul computer · sensibile", has(Screenshots.PERMISSION)) {
            ask.launch(arrayOf(Screenshots.PERMISSION))
        }
        Feature(Icons.Filled.DoNotDisturbOn, "Non disturbare dal computer", "Il computer può attivarlo anche sul telefono", Dnd.hasAccess(ctx)) {
            ctx.startActivity(Dnd.accessIntent())
        }
        Feature(Icons.Filled.Mic, "Microfono", "Per usare il telefono come microfono del computer · sensibile",
            has(android.Manifest.permission.RECORD_AUDIO)) { ask.launch(arrayOf(android.Manifest.permission.RECORD_AUDIO)) }
        Feature(Icons.Filled.Videocam, "Fotocamera", "Per webcam, foto e scanner richiesti dal computer · sensibile",
            has(android.Manifest.permission.CAMERA)) { ask.launch(arrayOf(android.Manifest.permission.CAMERA)) }
    }
    // Restart the phone features only when a grant actually changed.
    val state = listOf(listener, has(*Calls.PERMISSIONS), has(*Sms.PERMISSIONS), has(Screenshots.PERMISSION), Dnd.hasAccess(ctx))
    var seen by remember { mutableStateOf(state) }
    LaunchedEffect(state) {
        if (state != seen) {
            seen = state
            KonnektaService.refreshFeatures(ctx)
        }
    }
}

@Composable
private fun Feature(icon: androidx.compose.ui.graphics.vector.ImageVector, title: String, subtitle: String, on: Boolean, grant: () -> Unit) {
    ActionCard(icon, title, subtitle, trailing = { Switch(on, { if (!on) grant() }) }) { if (!on) grant() }
}

/** What a paired computer may do with this phone: one switch per permission. */
@Composable
fun DevicePermissionList(id: String) {
    val ui = LocalUi.current
    val scope = rememberCoroutineScope()
    val devices by Store.devices.collectAsState()
    var perms by remember { mutableStateOf(listOf<JSONObject>()) }
    LaunchedEffect(Unit) { core(ui.toast) { it.callArray("permissions") }?.let { perms = it.objects() } }
    val d = devices.firstOrNull { it.optString("id") == id } ?: return
    val granted = d.optJSONArray("permissions")?.let { a -> (0 until a.length()).map { a.optString(it) } }.orEmpty().toSet()
    Column {
        perms.forEach { p ->
            val pid = p.optString("id")
            Row(Modifier.fillMaxWidth().padding(vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(PERMISSION_LABELS[pid] ?: p.optString("label"))
                    if (p.optBoolean("sensitive")) Text("sensibile", style = MaterialTheme.typography.bodySmall, color = LocalStatus.current.warn)
                }
                Switch(checked = pid in granted, onCheckedChange = { on ->
                    scope.launch {
                        core(ui.toast) { it.call("setPermission", JSONObject().put("deviceId", id).put("permission", pid).put("granted", on)) }
                        core(ui.toast) { it.callArray("devices") }?.let { Store.devices.value = it.objects() }
                    }
                })
            }
        }
    }
}

/** One paired computer: state, what it may do with this phone, unpair. */
@Composable
fun DeviceScreen(id: String, onBack: () -> Unit) {
    val ui = LocalUi.current
    val scope = rememberCoroutineScope()
    val devices by Store.devices.collectAsState()
    val d = devices.firstOrNull { it.optString("id") == id } ?: return onBack()
    var confirm by remember { mutableStateOf(false) }

    SubHeader(d.optString("name"), d.optString("address").ifEmpty { null }, onBack)
    LazyColumn(Modifier.fillMaxSize()) {
        item {
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(vertical = 4.dp)) {
                StatusDot(d.optBoolean("connected"))
                Spacer(Modifier.width(8.dp))
                Text(if (d.optBoolean("connected")) "Connesso" else "Non raggiungibile", style = MaterialTheme.typography.bodyMedium)
            }
            d.optString("fingerprint").takeIf { it.isNotEmpty() }?.let {
                Text(it, style = MaterialTheme.typography.labelSmall.copy(fontFamily = FontFamily.Monospace),
                    color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        item {
            SectionTitle("Cosa può fare con questo telefono")
            Hint("Le funzioni sensibili restano spente finché non le attivi.")
        }
        item { DevicePermissionList(id) }
        item {
            OutlinedButton(onClick = { confirm = true }, Modifier.padding(vertical = 16.dp)) {
                Icon(Icons.Filled.LinkOff, null, tint = LocalStatus.current.danger)
                Spacer(Modifier.width(8.dp))
                Text("Dissocia", color = LocalStatus.current.danger)
            }
        }
    }
    if (confirm) AlertDialog(
        onDismissRequest = { confirm = false },
        title = { Text("Dissociare ${d.optString("name")}?") },
        text = { Text("Per ricollegarlo dovrai associarlo di nuovo e confrontare il codice.") },
        confirmButton = {
            TextButton(onClick = {
                confirm = false
                scope.launch {
                    core(ui.toast) { it.call("unpair", JSONObject().put("deviceId", id)) }
                    core(ui.toast) { it.callArray("devices") }?.let { Store.devices.value = it.objects() }
                    onBack()
                }
            }) { Text("Dissocia", color = LocalStatus.current.danger) }
        },
        dismissButton = { TextButton(onClick = { confirm = false }) { Text("Annulla") } },
    )
}
