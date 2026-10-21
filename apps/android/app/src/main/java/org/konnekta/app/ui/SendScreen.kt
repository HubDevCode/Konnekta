package org.konnekta.app.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.InsertDriveFile
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.ContentPasteGo
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.FolderOpen
import androidx.compose.material.icons.filled.NotificationsActive
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.UploadFile
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
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
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.GlobalScope
import kotlinx.coroutines.launch
import org.json.JSONObject
import org.konnekta.app.core.Types
import org.konnekta.app.service.Files
import org.konnekta.app.state.Store
import org.konnekta.app.state.supports

@Composable
fun SendScreen(current: JSONObject?, open: (Route) -> Unit) {
    val ctx = LocalContext.current
    val ui = LocalUi.current
    val scope = rememberCoroutineScope()
    val id = current?.optString("id")
    val online = current?.optBoolean("connected") == true
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (id == null || uris.isEmpty()) return@rememberLauncherForActivityResult
        sendUris(ctx, ui, id, uris)
    }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(vertical = 6.dp)) {
            StatusDot(online)
            Spacer(Modifier.width(8.dp))
            Text(
                when {
                    current == null -> "Nessun computer associato"
                    online -> "Invia a ${current.optString("name")}"
                    else -> "${current.optString("name")} non è raggiungibile"
                },
                style = MaterialTheme.typography.bodyMedium,
            )
        }
        ActionCard(Icons.Filled.ContentPasteGo, "Invia appunti", "Incollali sul computer", enabled = online, highlight = online) {
            val text = ui.clipboard()
            if (text.isNullOrEmpty()) ui.toast("Gli appunti sono vuoti")
            else scope.launch {
                core(ui.toast) { it.sendPacket(id!!, Types.CLIPBOARD, JSONObject().put("content", text)) }?.let { ui.toast("Appunti inviati") }
            }
        }
        SectionTitle("File")
        ActionCard(Icons.Filled.UploadFile, "Invia file", "Scegli dei file su questo telefono", enabled = online) {
            pick.launch(arrayOf("*/*"))
        }
        ActionCard(Icons.Filled.FolderOpen, "Prendi file", "Sfoglia le cartelle condivise del computer",
            enabled = online && current?.supports(Types.BROWSE_REQUEST) == true) { open(Route.Browse) }
        SectionTitle("Altro")
        ActionCard(Icons.Filled.NotificationsActive, "Ping", "Fai comparire un avviso sul computer", enabled = online) {
            scope.launch { send(ui.toast, id!!, Types.PING, JSONObject().put("message", "Ping dal telefono")) }
        }
        Hint("Per inviare testo, link o foto da un'altra app, sceglili e tocca Condividi › Konnekta.")
    }
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
