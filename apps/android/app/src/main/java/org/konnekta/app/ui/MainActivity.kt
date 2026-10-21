package org.konnekta.app.ui

import android.Manifest
import android.content.ClipboardManager
import android.os.Build
import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
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
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.core.Types
import org.konnekta.app.service.KonnektaService
import org.konnekta.app.service.formatKey

class MainActivity : ComponentActivity() {
    private val askNotifications = registerForActivityResult(ActivityResultContracts.RequestPermission()) {}

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            askNotifications.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
        KonnektaService.start(this)
        setContent { KonnektaTheme { App(::toast, ::clipboardText) } }
    }

    private fun toast(msg: String) = Toast.makeText(this, msg, Toast.LENGTH_SHORT).show()

    /** Android lets only the app on screen read the clipboard. */
    private fun clipboardText(): String? =
        getSystemService(ClipboardManager::class.java)?.primaryClip?.takeIf { it.itemCount > 0 }
            ?.getItemAt(0)?.coerceToText(this)?.toString()
}

/** Run a core call off the main thread; errors become a toast. */
private suspend fun <T> core(toast: (String) -> Unit, f: (org.konnekta.app.core.Core) -> T): T? =
    withContext(Dispatchers.IO) {
        val c = KonnektaApp.core ?: return@withContext null
        try {
            f(c)
        } catch (e: Exception) {
            withContext(Dispatchers.Main) { toast(e.message ?: "errore") }
            null
        }
    }

private fun JSONArray.objects(): List<JSONObject> = (0 until length()).mapNotNull { optJSONObject(it) }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun App(toast: (String) -> Unit, clipboard: () -> String?) {
    var devices by remember { mutableStateOf(listOf<JSONObject>()) }
    var nearby by remember { mutableStateOf(listOf<JSONObject>()) }
    var selected by remember { mutableStateOf<String?>(null) }
    var tick by remember { mutableStateOf(0) }
    val scope = rememberCoroutineScope()
    val prompt by KonnektaService.pairPrompt.collectAsState()

    // Refresh on core events and every few seconds.
    LaunchedEffect(Unit) { KonnektaApp.events.collect { tick++ } }
    LaunchedEffect(Unit) {
        while (true) {
            delay(3000)
            tick++
        }
    }
    LaunchedEffect(tick) {
        core(toast) { it.callArray("devices") }?.let { devices = it.objects() }
        core(toast) { it.callArray("discovered") }?.let { nearby = it.objects() }
    }

    prompt?.let { p -> PairDialog(p, toast) { KonnektaService.pairPrompt.value = null } }

    val dev = devices.firstOrNull { it.optString("id") == selected }
    Scaffold(topBar = { TopAppBar(title = { Text(dev?.optString("name") ?: "Konnekta") }) }) { pad ->
        Column(Modifier.padding(pad).padding(16.dp).fillMaxSize()) {
            if (dev != null) {
                BackHandler { selected = null }
                DeviceScreen(dev, toast, clipboard, onBack = { selected = null }, onChanged = { tick++ })
            } else {
                Home(devices, nearby, toast, onOpen = { selected = it }, onChanged = { tick++ }, scope = scope)
            }
        }
    }
}

@Composable
private fun Home(
    devices: List<JSONObject>,
    nearby: List<JSONObject>,
    toast: (String) -> Unit,
    onOpen: (String) -> Unit,
    onChanged: () -> Unit,
    scope: kotlinx.coroutines.CoroutineScope,
) {
    var address by remember { mutableStateOf("") }
    LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        val paired = devices.filter { it.optBoolean("paired") }
        item { Text("I tuoi computer", style = MaterialTheme.typography.titleMedium) }
        if (paired.isEmpty()) item { Text("Nessun computer associato: aprine uno qui sotto.", style = MaterialTheme.typography.bodySmall) }
        items(paired, key = { it.optString("id") }) { d ->
            Card(Modifier.fillMaxWidth().clickable { onOpen(d.optString("id")) }) {
                Column(Modifier.padding(12.dp)) {
                    Text(d.optString("name"), style = MaterialTheme.typography.titleSmall)
                    Text(if (d.optBoolean("connected")) "Connesso" else "Non raggiungibile", style = MaterialTheme.typography.bodySmall)
                }
            }
        }
        val others = devices.filter { !it.optBoolean("paired") } + nearby
        item { Spacer(Modifier.width(1.dp)); Text("Nelle vicinanze", style = MaterialTheme.typography.titleMedium) }
        if (others.isEmpty()) item { Text("Avvia Konnekta sul computer, nella stessa rete.", style = MaterialTheme.typography.bodySmall) }
        items(others.distinctBy { it.optString("id") }, key = { "n" + it.optString("id") }) { d ->
            Card(Modifier.fillMaxWidth()) {
                Row(Modifier.padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(d.optString("name"))
                        if (d.optString("protocol") != "konnekta" && d.has("protocol")) {
                            Text("tramite ${d.optString("protocol")}", style = MaterialTheme.typography.bodySmall)
                        }
                    }
                    Button(onClick = {
                        scope.launch {
                            val id = d.optString("id")
                            // Not connected yet: dial its address first.
                            if (!d.optBoolean("connected") && d.has("address")) {
                                core(toast) { it.call("connect", JSONObject().put("address", d.optString("address"))) }
                                delay(1500)
                            }
                            val r = core(toast) { it.callObject("pair", JSONObject().put("deviceId", id)) }
                            // Show our key while the computer asks its user to compare it.
                            r?.optString("verificationKey")?.takeIf { it.isNotEmpty() }?.let { key ->
                                KonnektaService.pairPrompt.value = JSONObject()
                                    .put("deviceId", id)
                                    .put("deviceName", d.optString("name"))
                                    .put("verificationKey", key)
                                    .put("waiting", true)
                            }
                            onChanged()
                        }
                    }) { Text("Associa") }
                }
            }
        }
        item {
            Row(verticalAlignment = Alignment.CenterVertically) {
                OutlinedTextField(address, { address = it }, Modifier.weight(1f), label = { Text("Indirizzo (anche Tailscale)") }, singleLine = true)
                Spacer(Modifier.width(8.dp))
                OutlinedButton(enabled = address.isNotBlank(), onClick = {
                    scope.launch {
                        core(toast) { it.call("connect", JSONObject().put("address", address.trim())) }
                        onChanged()
                    }
                }) { Text("Connetti") }
            }
        }
    }
}

@Composable
private fun PairDialog(p: JSONObject, toast: (String) -> Unit, onClose: () -> Unit) {
    val scope = rememberCoroutineScope()
    val id = p.optString("deviceId")
    val key = p.optString("verificationKey")
    val confirm = p.optBoolean("confirm")
    val name = p.optString("deviceName")
    if (p.optBoolean("waiting")) {
        // We asked: the answer arrives as pairConfirm / pairRejected, which replace or close this.
        AlertDialog(
            onDismissRequest = {},
            title = { Text("In attesa di $name") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("Il computer deve mostrare lo stesso codice:")
                    Text(formatKey(key), style = MaterialTheme.typography.headlineSmall)
                    Text("Accetta sul computer solo se tutti i 16 caratteri coincidono.", style = MaterialTheme.typography.bodySmall)
                }
            },
            confirmButton = {},
            dismissButton = {
                TextButton(onClick = {
                    scope.launch { core(toast) { it.call("rejectPair", JSONObject().put("deviceId", id).put("key", key)) } }
                    onClose()
                }) { Text("Annulla") }
            },
        )
        return
    }
    AlertDialog(
        onDismissRequest = {},
        title = { Text(if (confirm) "Conferma l'associazione" else "Associare $name?") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Il computer deve mostrare lo stesso codice:")
                Text(formatKey(key), style = MaterialTheme.typography.headlineSmall)
                Text("Confronta tutti i 16 caratteri. Se sono diversi, rifiuta.", style = MaterialTheme.typography.bodySmall)
            }
        },
        confirmButton = {
            TextButton(onClick = {
                scope.launch { core(toast) { it.call("acceptPair", JSONObject().put("deviceId", id).put("key", key)) } }
                onClose()
            }) { Text("Il codice è uguale") }
        },
        dismissButton = {
            TextButton(onClick = {
                scope.launch { core(toast) { it.call("rejectPair", JSONObject().put("deviceId", id).put("key", key)) } }
                onClose()
            }) { Text("Rifiuta") }
        },
    )
}

@Composable
private fun DeviceScreen(d: JSONObject, toast: (String) -> Unit, clipboard: () -> String?, onBack: () -> Unit, onChanged: () -> Unit) {
    val scope = rememberCoroutineScope()
    val id = d.optString("id")
    var perms by remember { mutableStateOf(listOf<JSONObject>()) }
    LaunchedEffect(Unit) { core(toast) { it.callArray("permissions") }?.let { perms = it.objects() } }
    val granted = d.optJSONArray("permissions")?.let { a -> (0 until a.length()).map { a.optString(it) }.toSet() } ?: emptySet()
    LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        item {
            Text(if (d.optBoolean("connected")) "Connesso · ${d.optString("address")}" else "Non raggiungibile", style = MaterialTheme.typography.bodySmall)
        }
        item {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(enabled = d.optBoolean("connected"), onClick = {
                    scope.launch { core(toast) { it.sendPacket(id, Types.PING, JSONObject().put("message", "Ping dal telefono")) } }
                }) { Text("Ping") }
                Button(enabled = d.optBoolean("connected"), onClick = {
                    val text = clipboard()
                    if (text.isNullOrEmpty()) {
                        toast("Gli appunti sono vuoti")
                    } else {
                        scope.launch {
                            core(toast) { it.sendPacket(id, Types.CLIPBOARD, JSONObject().put("content", text)) }?.let { toast("Appunti inviati") }
                        }
                    }
                }) { Text("Invia appunti") }
            }
        }
        item { Text("Permessi di questo computer", style = MaterialTheme.typography.titleMedium) }
        items(perms, key = { it.optString("id") }) { p ->
            val pid = p.optString("id")
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(p.optString("label"))
                    if (p.optBoolean("sensitive")) Text("sensibile", style = MaterialTheme.typography.bodySmall)
                }
                Switch(checked = pid in granted, onCheckedChange = { on ->
                    scope.launch {
                        core(toast) { it.call("setPermission", JSONObject().put("deviceId", id).put("permission", pid).put("granted", on)) }
                        onChanged()
                    }
                })
            }
        }
        item {
            OutlinedButton(onClick = {
                scope.launch {
                    core(toast) { it.call("unpair", JSONObject().put("deviceId", id)) }
                    onBack()
                    onChanged()
                }
            }) { Text("Disassocia") }
        }
    }
}
