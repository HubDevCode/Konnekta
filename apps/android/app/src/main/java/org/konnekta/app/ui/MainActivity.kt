package org.konnekta.app.ui

import android.Manifest
import android.content.ClipboardManager
import android.os.Build
import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Devices
import androidx.compose.material.icons.filled.Inbox
import androidx.compose.material.icons.filled.Tune
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.service.KonnektaService
import org.konnekta.app.state.Store

/** UI helpers handed down to every screen. */
class Ui(val toast: (String) -> Unit, val clipboard: () -> String?)

val LocalUi = staticCompositionLocalOf<Ui> { error("no Ui") }

class MainActivity : ComponentActivity() {
    private val askNotifications = registerForActivityResult(ActivityResultContracts.RequestPermission()) {}

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            askNotifications.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
        Store.load(this)
        KonnektaService.start(this)
        val ui = Ui(::toast, ::clipboardText)
        setContent {
            KonnektaTheme {
                CompositionLocalProvider(LocalUi provides ui) { App() }
            }
        }
    }

    private fun toast(msg: String) = Toast.makeText(this, msg, Toast.LENGTH_SHORT).show()

    /** Android lets only the app on screen read the clipboard. */
    private fun clipboardText(): String? =
        getSystemService(ClipboardManager::class.java)?.primaryClip?.takeIf { it.itemCount > 0 }
            ?.getItemAt(0)?.coerceToText(this)?.toString()
}

enum class Tab(val label: String, val icon: ImageVector) {
    Inbox("Attività", Icons.Filled.Inbox),
    Send("Invia", Icons.AutoMirrored.Filled.Send),
    Control("Controllo", Icons.Filled.Tune),
    Computers("Computer", Icons.Filled.Devices),
}

/** Screens opened from a tab. */
sealed interface Route {
    data object Media : Route
    data object Touchpad : Route
    data object Presenter : Route
    data object Commands : Route
    data object Deck : Route
    data object Browse : Route
    data class Device(val id: String) : Route
}

@Composable
private fun App() {
    val ui = LocalUi.current
    val devices by Store.devices.collectAsState()
    val selected by Store.selected.collectAsState()
    val inbox by Store.inbox.collectAsState()
    val prompt by KonnektaService.pairPrompt.collectAsState()
    var tab by rememberSaveable { mutableStateOf(Tab.Inbox) }
    var route by remember { mutableStateOf<Route?>(null) }

    // Keep the device list fresh while the app is open.
    LaunchedEffect(Unit) {
        while (true) {
            core(ui.toast) { it.callArray("devices") }?.let { Store.devices.value = it.objects() }
            core(ui.toast) { it.callArray("discovered") }?.let { Store.nearby.value = it.objects() }
            delay(3000)
        }
    }
    LaunchedEffect(Unit) { KonnektaApp.events.collect { e ->
        if (e.optString("event") in setOf("deviceConnected", "deviceDisconnected", "deviceUpdated", "paired", "unpaired")) {
            core(ui.toast) { it.callArray("devices") }?.let { Store.devices.value = it.objects() }
        }
    } }
    // First run: nothing paired yet, start on the Computers tab.
    LaunchedEffect(devices.isNotEmpty()) {
        if (devices.none { it.optBoolean("paired") } && tab == Tab.Inbox) tab = Tab.Computers
    }

    val current = remember(devices, selected) { Store.current() }
    prompt?.let { p -> PairSheet(p) { KonnektaService.pairPrompt.value = null } }

    route?.let { r ->
        BackHandler { route = null }
        Column(Modifier.fillMaxSize().safeDrawingPadding().padding(horizontal = 16.dp)) {
            val back = { route = null }
            when (r) {
                Route.Media -> MediaScreen(current, back)
                Route.Touchpad -> TouchpadScreen(current, back)
                Route.Presenter -> PresenterScreen(current, back)
                Route.Commands -> CommandsScreen(current, back)
                Route.Deck -> DeckScreen(current, back)
                Route.Browse -> BrowseScreen(current, back)
                is Route.Device -> DeviceScreen(r.id, back)
            }
        }
        return
    }

    Scaffold(
        bottomBar = {
            NavigationBar {
                Tab.entries.forEach { t ->
                    NavigationBarItem(
                        selected = tab == t,
                        onClick = { tab = t },
                        icon = {
                            if (t == Tab.Inbox && inbox.isNotEmpty()) {
                                BadgedBox(badge = { Badge { Text(inbox.size.coerceAtMost(99).toString()) } }) { Icon(t.icon, null) }
                            } else {
                                Icon(t.icon, null)
                            }
                        },
                        label = { Text(t.label) },
                    )
                }
            }
        },
    ) { pad ->
        Column(Modifier.fillMaxSize().padding(pad).statusBarsPadding().padding(horizontal = 16.dp)) {
            Row(Modifier.fillMaxWidth().padding(top = 12.dp, bottom = 4.dp)) { ScopeChip(devices, current) }
            val open = { r: Route -> route = r }
            when (tab) {
                Tab.Inbox -> InboxScreen(current, inbox, open)
                Tab.Send -> SendScreen(current, open)
                Tab.Control -> ControlScreen(current, open)
                Tab.Computers -> ComputersScreen(devices, open)
            }
        }
    }
}

/**
 * Pairing sheet. `waiting`: we asked and the computer must confirm;
 * otherwise the computer asked (or confirmed) and we accept or refuse.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PairSheet(p: JSONObject, onClose: () -> Unit) {
    val ui = LocalUi.current
    val scope = rememberCoroutineScope()
    val id = p.optString("deviceId")
    val key = p.optString("verificationKey")
    val name = p.optString("deviceName").ifBlank { Store.name(id) }
    val waiting = p.optBoolean("waiting")
    val state = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    fun answer(method: String) {
        scope.launch { core(ui.toast) { it.call(method, JSONObject().put("deviceId", id).put("key", key)) } }
        onClose()
    }
    ModalBottomSheet(onDismissRequest = {}, sheetState = state) {
        Column(Modifier.padding(horizontal = 20.dp).padding(bottom = 28.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text(if (waiting) "In attesa" else "Associa", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary)
            Text(name, style = MaterialTheme.typography.headlineSmall)
            Text(
                if (waiting) "Accetta sul computer solo se mostra lo stesso codice. Confronta tutti i 16 caratteri."
                else "Controlla che $name mostri lo stesso codice. Confronta tutti i 16 caratteri.",
                style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            KeyBlocks(key)
            Spacer(Modifier.height(4.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                OutlinedButton(onClick = { answer("rejectPair") }, Modifier.weight(1f)) { Text(if (waiting) "Annulla" else "Rifiuta") }
                if (!waiting) Button(onClick = { answer("acceptPair") }, Modifier.weight(2f)) { Text("Associa") }
            }
        }
    }
}
