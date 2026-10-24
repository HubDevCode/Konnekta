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
import androidx.compose.foundation.background
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
import android.content.res.Configuration
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.material3.NavigationRail
import androidx.compose.material3.NavigationRailItem
import androidx.compose.ui.platform.LocalConfiguration
import org.konnekta.app.state.Settings

/** Set while the Presentation screen is open: the computer the volume keys drive. */
object PresenterKeys {
    @Volatile
    var deviceId: String? = null
}

/** UI helpers handed down to every screen. */
class Ui(val toast: (String) -> Unit, val clipboard: () -> String?)

val LocalUi = staticCompositionLocalOf<Ui> { error("no Ui") }

class MainActivity : ComponentActivity() {
    private val askNotifications = registerForActivityResult(ActivityResultContracts.RequestPermission()) {}
    private val askBluetooth = registerForActivityResult(ActivityResultContracts.RequestPermission()) { KonnektaService.refreshFeatures(this) }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            askNotifications.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
        Store.load(this)
        if (org.konnekta.app.state.Settings.bluetooth.value && !org.konnekta.app.service.BluetoothBridge.allowed(this)) {
            askBluetooth.launch(Manifest.permission.BLUETOOTH_CONNECT)
        }
        KonnektaService.start(this)
        val ui = Ui(::toast, ::clipboardText)
        handle(intent)
        setContent {
            KonnektaTheme {
                CompositionLocalProvider(LocalUi provides ui) { App() }
            }
        }
    }

    private fun toast(msg: String) = Toast.makeText(this, msg, Toast.LENGTH_SHORT).show()

    override fun onNewIntent(intent: android.content.Intent) {
        super.onNewIntent(intent)
        handle(intent)
    }

    /** "Accetta" in a stream-request notification opens the app to finish the start. */
    private fun handle(i: android.content.Intent?) {
        if (i?.action == ACTION_ACCEPT_STREAM) {
            val dev = i.getStringExtra("device") ?: return
            val plugin = i.getStringExtra("plugin") ?: return
            NotificationAccept.stream.value = dev to plugin
        }
    }

    companion object {
        const val ACTION_ACCEPT_STREAM = "org.konnekta.app.ACCEPT_STREAM_UI"
    }

    /** In Presentation the volume keys change slide (up = next, down = previous). */
    override fun dispatchKeyEvent(event: android.view.KeyEvent): Boolean {
        val target = PresenterKeys.deviceId
        val code = event.keyCode
        if (target != null && (code == android.view.KeyEvent.KEYCODE_VOLUME_UP || code == android.view.KeyEvent.KEYCODE_VOLUME_DOWN)) {
            if (event.action == android.view.KeyEvent.ACTION_DOWN && event.repeatCount == 0) {
                val cmd = if (code == android.view.KeyEvent.KEYCODE_VOLUME_UP) "next" else "previous"
                Thread { runCatching { KonnektaApp.core?.sendPacket(target, org.konnekta.app.core.Types.PRESENTER, JSONObject().put("konnektaKey", cmd)) } }.start()
            }
            return true
        }
        return super.dispatchKeyEvent(event)
    }

    /** Android lets only the app on screen read the clipboard. */
    private fun clipboardText(): String? =
        getSystemService(ClipboardManager::class.java)?.primaryClip?.takeIf { it.itemCount > 0 }
            ?.getItemAt(0)?.coerceToText(this)?.toString()
}

enum class Tab(val label: String, val icon: ImageVector) {
    Control("Controllo", Icons.Filled.Tune),
    Send("Invia", Icons.AutoMirrored.Filled.Send),
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
    data object Desktop : Route
    data object Settings : Route
    data object Camera : Route
    data object Extend : Route
    data object About : Route
    data class Device(val id: String) : Route
}

/**
 * Navigation state held by the process, not by the composition: rotating
 * the phone (even when Android rebuilds the activity) keeps the open
 * feature and the tab.
 */
object Nav {
    val route = kotlinx.coroutines.flow.MutableStateFlow<Route?>(null)
    var tab = 0
}

@Composable
private fun App() {
    val ui = LocalUi.current
    val devices by Store.devices.collectAsState()
    val selected by Store.selected.collectAsState()
    val prompt by KonnektaService.pairPrompt.collectAsState()
    val route by Nav.route.collectAsState()

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

    val current = remember(devices, selected) { Store.current() }
    prompt?.let { p -> PairSheet(p) { KonnektaService.pairPrompt.value = null } }

    val ctx = androidx.compose.ui.platform.LocalContext.current
    var onboarding by remember { mutableStateOf(Onboarding.needed(ctx)) }
    if (onboarding) {
        Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
            OnboardingScreen { onboarding = false }
        }
        return
    }

    // Over every screen, also inside a feature.
    RequestDialogs()
    StreamProblemDialog()
    UpdateDialog()
    // A new version on GitHub: asked once per start, like the desktop app.
    LaunchedEffect(Unit) { org.konnekta.app.service.Updates.checkOnStart(ctx) }

    route?.let { r ->
        BackHandler { Nav.route.value = null }
        // The second screen uses the whole display, bars and cut-outs included.
        if (r == Route.Extend) {
            ExtendScreen(current) { Nav.route.value = null }
            return
        }
        Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).safeDrawingPadding().padding(horizontal = 16.dp)) {
            val back = { Nav.route.value = null }
            when (r) {
                Route.Media -> MediaScreen(current, back)
                Route.Touchpad -> TouchpadScreen(current, back)
                Route.Presenter -> PresenterScreen(current, back)
                Route.Commands -> CommandsScreen(current, back)
                Route.Deck -> DeckScreen(current, back)
                Route.Browse -> BrowseScreen(current, back)
                Route.Desktop -> RemoteDesktopScreen(current, back)
                Route.Settings -> SettingsScreen(back)
                Route.Camera -> CameraScreen(current, back)
                Route.Extend -> {}
                Route.About -> AboutScreen(back)
                is Route.Device -> DeviceScreen(r.id, back)
            }
        }
        return
    }

    // Tabs follow the swipe; tapping a tab scrolls the pager.
    val pager = androidx.compose.foundation.pager.rememberPagerState(initialPage = Nav.tab, pageCount = { Tab.entries.size })
    LaunchedEffect(pager.currentPage) { Nav.tab = pager.currentPage }
    // "Personalizza" from the settings: show the page being arranged.
    val editing by Settings.editing.collectAsState()
    LaunchedEffect(editing) {
        when (editing) {
            "control" -> pager.animateScrollToPage(Tab.Control.ordinal)
            "send" -> pager.animateScrollToPage(Tab.Send.ordinal)
        }
    }
    val scope = rememberCoroutineScope()
    val landscape = LocalConfiguration.current.orientation == Configuration.ORIENTATION_LANDSCAPE
    val navSide by Settings.navLandscape.collectAsState()
    val topBar by Settings.topBarLandscape.collectAsState()
    val side = if (landscape) navSide else Settings.NavSide.Bottom
    val goTo = { t: Tab -> scope.launch { pager.animateScrollToPage(t.ordinal) }; Unit }
    val open = { r: Route -> Nav.route.value = r }

    val content: @Composable (Modifier) -> Unit = { mod ->
        Column(mod) {
            when {
                !landscape || topBar == Settings.TopBar.Full ->
                    Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 4.dp)) { ScopeChip(devices, current) }
                topBar == Settings.TopBar.Compact ->
                    Row(Modifier.fillMaxWidth().padding(start = 12.dp, end = 12.dp, top = 4.dp)) { ScopeChip(devices, current, compact = true) }
            }
            androidx.compose.foundation.pager.HorizontalPager(pager, Modifier.fillMaxSize(), beyondViewportPageCount = 1) { page ->
                Column(Modifier.fillMaxSize().padding(horizontal = 16.dp)) {
                    when (Tab.entries[page]) {
                        Tab.Control -> ControlScreen(current, open)
                        Tab.Send -> SendScreen(current, open)
                        Tab.Computers -> ComputersScreen(devices, open)
                    }
                }
            }
        }
    }

    if (side == Settings.NavSide.Bottom) {
        Scaffold(
            bottomBar = {
                NavigationBar {
                    Tab.entries.forEach { t ->
                        NavigationBarItem(
                            selected = pager.currentPage == t.ordinal,
                            onClick = { goTo(t) },
                            icon = { Icon(t.icon, null) },
                            label = { Text(t.label) },
                        )
                    }
                }
            },
        ) { pad -> content(Modifier.fillMaxSize().padding(pad)) }
        return
    }
    // Landscape: the tabs on one side free the height for the content.
    val rail: @Composable () -> Unit = {
        NavigationRail(Modifier.fillMaxHeight()) {
            Spacer(Modifier.weight(1f))
            Tab.entries.forEach { t ->
                NavigationRailItem(
                    selected = pager.currentPage == t.ordinal,
                    onClick = { goTo(t) },
                    icon = { Icon(t.icon, null) },
                    label = { Text(t.label) },
                )
            }
            Spacer(Modifier.weight(1f))
        }
    }
    Row(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).safeDrawingPadding()) {
        if (side == Settings.NavSide.Left) rail()
        content(Modifier.weight(1f).fillMaxHeight())
        if (side == Settings.NavSide.Right) rail()
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
