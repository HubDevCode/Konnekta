package org.konnekta.app.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items as gridItems
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.automirrored.filled.KeyboardReturn
import androidx.compose.material.icons.automirrored.filled.VolumeDown
import androidx.compose.material.icons.automirrored.filled.VolumeUp
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.ArrowForward
import androidx.compose.material.icons.automirrored.filled.VolumeOff
import androidx.compose.material.icons.filled.Backspace
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.ContentPaste
import androidx.compose.material.icons.filled.DoNotDisturbOn
import androidx.compose.material.icons.filled.FiberManualRecord
import androidx.compose.material.icons.filled.HeadsetOff
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Looks3
import androidx.compose.material.icons.filled.LooksOne
import androidx.compose.material.icons.filled.LooksTwo
import androidx.compose.material.icons.filled.MicOff
import androidx.compose.material.icons.filled.RecordVoiceOver
import androidx.compose.material.icons.filled.Screenshot
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Sensors
import androidx.compose.material.icons.filled.SportsEsports
import androidx.compose.material.icons.filled.Tab
import androidx.compose.material.icons.filled.Videocam
import androidx.compose.material.icons.filled.Work
import androidx.compose.material.icons.filled.DarkMode
import androidx.compose.material.icons.filled.Dashboard
import androidx.compose.material.icons.filled.DesktopWindows
import androidx.compose.material.icons.filled.Speaker
import androidx.compose.material.icons.filled.MusicNote
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.PlayCircle
import androidx.compose.material.icons.filled.SkipNext
import androidx.compose.material.icons.filled.SkipPrevious
import androidx.compose.material.icons.filled.Slideshow
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material.icons.filled.Terminal
import androidx.compose.material.icons.filled.TouchApp
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.launch
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.core.Types
import org.konnekta.app.service.Streams
import org.konnekta.app.state.Store
import org.konnekta.app.state.supports
import kotlin.math.abs

@Composable
fun ControlScreen(current: JSONObject?, open: (Route) -> Unit) {
    val online = current?.optBoolean("connected") == true
    fun can(type: String) = online && current?.supports(type) == true
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
        if (current == null) {
            Hint("Associa un computer per controllarlo da qui.")
            return
        }
        if (!online) Hint("${current.optString("name")} non è raggiungibile: i comandi si attivano quando si collega.")
        SectionTitle("Computer")
        ActionCard(Icons.Filled.MusicNote, "Media", "Musica e video in riproduzione, volume", enabled = can(Types.MEDIA_REQUEST)) { open(Route.Media) }
        ActionCard(Icons.Filled.TouchApp, "Touchpad e tastiera", "Muovi il puntatore e scrivi sul computer", enabled = can(Types.INPUT)) { open(Route.Touchpad) }
        ActionCard(Icons.Filled.Slideshow, "Presentazione", "Avanti, indietro e puntatore laser", enabled = can(Types.PRESENTER)) { open(Route.Presenter) }
        ActionCard(Icons.Filled.DesktopWindows, "Desktop remoto", "Vedi e controlla lo schermo del computer", enabled = can(Types.DESKTOP)) { open(Route.Desktop) }
        val live by Streams.live.collectAsState()
        val audioOn = live.containsKey("systemaudio/${current.optString("id")}")
        ActionCard(
            Icons.Filled.Speaker, "Audio del computer", if (audioOn) "In ascolto sul telefono: tocca per fermare" else "Ascolta sul telefono l'audio del computer",
            enabled = can(Types.SYSTEM_AUDIO), highlight = audioOn,
        ) {
            val id = current.optString("id")
            if (audioOn) Thread { Streams.stop(id, "systemaudio") }.start() else Streams.request(id, "systemaudio")
        }
        SectionTitle("Automazioni")
        ActionCard(Icons.Filled.Dashboard, "Mobile Deck", "I pulsanti configurati sul computer", enabled = can(Types.DECK_REQUEST)) { open(Route.Deck) }
        ActionCard(Icons.Filled.Terminal, "Comandi", "Avvia i comandi personalizzati del computer", enabled = can(Types.COMMANDS_REQUEST)) { open(Route.Commands) }
        Hint("Le voci spente richiedono il permesso corrispondente sul computer (Dispositivi › Permessi).")
    }
}

// --- media ---------------------------------------------------------------------

@Composable
fun MediaScreen(current: JSONObject?, onBack: () -> Unit) {
    val ui = LocalUi.current
    val scope = rememberCoroutineScope()
    val id = current?.optString("id") ?: return onBack()
    val media by Store.pcMedia.collectAsState()
    val players by Store.pcPlayers.collectAsState()
    val m = media[id] ?: JSONObject()
    val list = players[id].orEmpty()
    var player by remember { mutableStateOf<String?>(null) }
    val active = player ?: m.optString("player").ifEmpty { null } ?: list.firstOrNull()
    fun req(b: JSONObject) = scope.launch { send(ui.toast, id, Types.MEDIA_REQUEST, b.apply { active?.let { put("player", it) } }) }
    LaunchedEffect(id) { req(JSONObject().put("requestPlayerList", true).put("requestNowPlaying", true).put("requestVolume", true)) }

    SubHeader("Media", current.optString("name"), onBack)
    if (list.size > 1) {
        LazyRow(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.padding(bottom = 8.dp)) {
            items(list) { p ->
                FilterChip(selected = p == active, onClick = {
                    player = p
                    req(JSONObject().put("requestNowPlaying", true))
                }, label = { Text(p, fontFamily = FontFamily.Monospace) })
            }
        }
    }
    val playing = m.optBoolean("isPlaying")
    val length = m.optLong("length")
    val pos = m.optLong("pos")
    Surface(shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surfaceContainerLow,
        border = BorderStroke(1.dp, LocalStatus.current.border), modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(16.dp)) {
            Text(m.optString("title").ifEmpty { "Niente in riproduzione" }, style = MaterialTheme.typography.headlineSmall)
            Text(listOf(m.optString("artist"), m.optString("album")).filter { it.isNotEmpty() }.joinToString(" · "),
                color = MaterialTheme.colorScheme.onSurfaceVariant)
            active?.let { Text(it, style = MaterialTheme.typography.labelSmall.copy(fontFamily = FontFamily.Monospace),
                color = MaterialTheme.colorScheme.onSurfaceVariant) }
            if (length > 0) {
                var seek by remember(pos) { mutableFloatStateOf(pos.toFloat()) }
                Slider(seek, { seek = it }, valueRange = 0f..length.toFloat(),
                    onValueChangeFinished = { req(JSONObject().put("SetPosition", seek.toLong())) })
                Row {
                    Text(clock(seek.toLong()), style = MaterialTheme.typography.labelSmall.copy(fontFamily = FontFamily.Monospace))
                    Spacer(Modifier.weight(1f))
                    Text(clock(length), style = MaterialTheme.typography.labelSmall.copy(fontFamily = FontFamily.Monospace))
                }
            }
        }
    }
    Row(Modifier.fillMaxWidth().padding(vertical = 12.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        BigButton(Icons.Filled.SkipPrevious, Modifier.weight(1f)) { req(JSONObject().put("action", "Previous")) }
        BigButton(if (playing) Icons.Filled.Pause else Icons.Filled.PlayArrow, Modifier.weight(1f), primary = true) {
            req(JSONObject().put("action", "PlayPause"))
        }
        BigButton(Icons.Filled.SkipNext, Modifier.weight(1f)) { req(JSONObject().put("action", "Next")) }
    }
    if (m.has("volume")) {
        var vol by remember(m.optInt("volume")) { mutableFloatStateOf(m.optInt("volume").toFloat()) }
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.AutoMirrored.Filled.VolumeDown, null)
            Slider(vol, { vol = it }, Modifier.weight(1f), valueRange = 0f..100f,
                onValueChangeFinished = { req(JSONObject().put("setVolume", vol.toInt())) })
            Icon(Icons.AutoMirrored.Filled.VolumeUp, null)
        }
    }
}

private fun clock(ms: Long): String {
    val s = ms / 1000
    return "%d:%02d".format(s / 60, s % 60)
}

@Composable
private fun BigButton(icon: ImageVector, modifier: Modifier, primary: Boolean = false, onClick: () -> Unit) {
    val haptic = LocalHapticFeedback.current
    Surface(
        onClick = {
            haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
            onClick()
        },
        shape = RoundedCornerShape(14.dp),
        color = if (primary) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceContainerHigh,
        modifier = modifier.height(64.dp),
    ) {
        Box(contentAlignment = Alignment.Center) {
            Icon(icon, null, Modifier.size(30.dp), tint = if (primary) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface)
        }
    }
}

// --- touchpad --------------------------------------------------------------------

/** Special key codes of konnekta.input (same as the KDE Connect mousepad). */
private object Key {
    const val BACKSPACE = 1
    const val TAB = 2
    const val LEFT = 4
    const val UP = 5
    const val RIGHT = 6
    const val DOWN = 7
    const val ENTER = 12
    const val ESC = 14
}

@Composable
fun ColumnScope.TouchpadScreen(current: JSONObject?, onBack: () -> Unit) {
    val id = current?.optString("id") ?: return onBack()
    // One queue keeps packets in order without blocking the gesture handlers.
    val queue = remember { Channel<JSONObject>(Channel.UNLIMITED) }
    LaunchedEffect(id) {
        kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) {
            for (b in queue) runCatching { KonnektaApp.core?.sendPacket(id, Types.INPUT, b) }
        }
    }
    var ctrl by remember { mutableStateOf(false) }
    var alt by remember { mutableStateOf(false) }
    var shift by remember { mutableStateOf(false) }
    var meta by remember { mutableStateOf(false) }
    fun mods(b: JSONObject) = b.apply {
        if (ctrl) put("ctrl", true)
        if (alt) put("alt", true)
        if (shift) put("shift", true)
        if (meta) put("super", true)
    }
    fun emit(b: JSONObject) { queue.trySend(b) }
    fun special(code: Int) = emit(mods(JSONObject().put("specialKey", code)))
    val sensitivity = 1.6f

    SubHeader("Touchpad e tastiera", current.optString("name"), onBack)
    Surface(
        shape = RoundedCornerShape(16.dp),
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        border = BorderStroke(1.dp, LocalStatus.current.border),
        modifier = Modifier.fillMaxWidth().weight(1f)
            .pointerInput(id) { touchpad(sensitivity, ::emit) },
    ) {
        Box(contentAlignment = Alignment.Center) {
            Text("1 dito muove · tocco = clic\n2 dita scorrono · tocco con 2 dita = tasto destro\ntieni premuto per trascinare",
                textAlign = TextAlign.Center, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
    Row(Modifier.fillMaxWidth().padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        KeyCap("sinistro", Modifier.weight(1f)) { emit(JSONObject().put("singleclick", true)) }
        KeyCap("centrale", Modifier.weight(1f)) { emit(JSONObject().put("middleclick", true)) }
        KeyCap("destro", Modifier.weight(1f)) { emit(JSONObject().put("rightclick", true)) }
    }
    Row(Modifier.fillMaxWidth().padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        KeyCap("esc", Modifier.weight(1f)) { special(Key.ESC) }
        KeyCap("tab", Modifier.weight(1f)) { special(Key.TAB) }
        KeyCap("←", Modifier.weight(1f)) { special(Key.LEFT) }
        KeyCap("↑", Modifier.weight(1f)) { special(Key.UP) }
        KeyCap("↓", Modifier.weight(1f)) { special(Key.DOWN) }
        KeyCap("→", Modifier.weight(1f)) { special(Key.RIGHT) }
    }
    Row(Modifier.fillMaxWidth().padding(top = 6.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        KeyCap("ctrl", Modifier.weight(1f), ctrl) { ctrl = !ctrl }
        KeyCap("alt", Modifier.weight(1f), alt) { alt = !alt }
        KeyCap("shift", Modifier.weight(1f), shift) { shift = !shift }
        KeyCap("super", Modifier.weight(1f), meta) { meta = !meta }
        Surface(onClick = { special(Key.BACKSPACE) }, shape = RoundedCornerShape(10.dp),
            color = MaterialTheme.colorScheme.surfaceContainerHigh, modifier = Modifier.weight(1f).height(44.dp)) {
            Box(contentAlignment = Alignment.Center) { Icon(Icons.Filled.Backspace, "Cancella", Modifier.size(18.dp)) }
        }
    }
    var text by remember { mutableStateOf("") }
    Row(Modifier.fillMaxWidth().padding(vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        OutlinedTextField(
            text,
            { v ->
                // Diff against the previous value: typed characters or backspaces.
                if (v.length > text.length && v.startsWith(text)) emit(mods(JSONObject().put("key", v.substring(text.length))))
                else if (v.length < text.length) repeat(text.length - v.length) { special(Key.BACKSPACE) }
                text = if (v.length > 64) "" else v
            },
            Modifier.weight(1f),
            placeholder = { Text("Scrivi su ${current.optString("name")}") },
            singleLine = true,
            keyboardOptions = KeyboardOptions(autoCorrectEnabled = false, imeAction = ImeAction.Send),
            keyboardActions = KeyboardActions(onSend = {
                special(Key.ENTER)
                text = ""
            }),
        )
        Spacer(Modifier.size(8.dp))
        Surface(onClick = { special(Key.ENTER) }, shape = RoundedCornerShape(12.dp),
            color = MaterialTheme.colorScheme.surfaceContainerHigh, modifier = Modifier.size(56.dp)) {
            Box(contentAlignment = Alignment.Center) { Icon(Icons.AutoMirrored.Filled.KeyboardReturn, "Invio") }
        }
    }
}

@Composable
private fun KeyCap(label: String, modifier: Modifier, on: Boolean = false, onClick: () -> Unit) {
    Surface(
        onClick = onClick,
        shape = RoundedCornerShape(10.dp),
        color = if (on) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceContainerHigh,
        border = if (on) BorderStroke(1.dp, MaterialTheme.colorScheme.primary) else null,
        modifier = modifier.height(44.dp),
    ) {
        Box(contentAlignment = Alignment.Center) {
            Text(label, fontFamily = FontFamily.Monospace, fontSize = 13.sp, maxLines = 1)
        }
    }
}

/**
 * Touchpad gestures: one finger moves (tap = click, double tap = double
 * click, hold still = drag), two fingers scroll (tap = right click).
 */
private suspend fun androidx.compose.ui.input.pointer.PointerInputScope.touchpad(sens: Float, emit: (JSONObject) -> Unit) {
    var lastTapUp = 0L
    awaitEachGesture {
        val down = awaitFirstDown()
        val start = down.uptimeMillis
        var maxPointers = 1
        var moved = 0f
        var dragging = false
        var pendX = 0f
        var pendY = 0f
        while (true) {
            val ev = awaitPointerEvent()
            val pressed = ev.changes.filter { it.pressed }
            maxPointers = maxOf(maxPointers, pressed.size)
            if (pressed.isEmpty()) break
            if (!dragging && maxPointers == 1 && moved < 8f && ev.changes.first().uptimeMillis - start > 450) {
                dragging = true
                emit(JSONObject().put("singlehold", true))
            }
            if (ev.type == PointerEventType.Move) {
                val c = pressed.first()
                val dx = c.position.x - c.previousPosition.x
                val dy = c.position.y - c.previousPosition.y
                moved += abs(dx) + abs(dy)
                pendX += dx
                pendY += dy
                // Coalesce tiny moves: fewer packets, same path.
                if (abs(pendX) + abs(pendY) >= 2f) {
                    if (pressed.size >= 2) emit(JSONObject().put("dx", pendX / 2).put("dy", pendY / 2).put("scroll", true))
                    else emit(JSONObject().put("dx", pendX * sens).put("dy", pendY * sens))
                    pendX = 0f
                    pendY = 0f
                }
                ev.changes.forEach { it.consume() }
            }
        }
        val now = System.currentTimeMillis()
        when {
            dragging -> emit(JSONObject().put("singlerelease", true))
            moved < 12f && maxPointers >= 2 -> emit(JSONObject().put("rightclick", true))
            moved < 12f -> {
                if (now - lastTapUp < 300) emit(JSONObject().put("doubleclick", true)) else emit(JSONObject().put("singleclick", true))
                lastTapUp = now
            }
        }
    }
}

// --- presenter -------------------------------------------------------------------

@Composable
fun ColumnScope.PresenterScreen(current: JSONObject?, onBack: () -> Unit) {
    val ui = LocalUi.current
    val scope = rememberCoroutineScope()
    val id = current?.optString("id") ?: return onBack()
    fun cmd(c: String) = scope.launch { send(ui.toast, id, Types.PRESENTER, JSONObject().put("konnektaKey", c)) }
    SubHeader("Presentazione", current.optString("name"), onBack)
    Row(Modifier.fillMaxWidth().height(220.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        BigTile(Icons.AutoMirrored.Filled.KeyboardArrowLeft, "Indietro", Modifier.weight(1f).fillMaxSize()) { cmd("previous") }
        BigTile(Icons.AutoMirrored.Filled.KeyboardArrowRight, "Avanti", Modifier.weight(1f).fillMaxSize(), primary = true) { cmd("next") }
    }
    Row(Modifier.fillMaxWidth().height(90.dp).padding(top = 10.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        BigTile(Icons.Filled.PlayArrow, "Avvia", Modifier.weight(1f).fillMaxSize()) { cmd("start") }
        BigTile(Icons.Filled.Stop, "Esci", Modifier.weight(1f).fillMaxSize()) { cmd("stop") }
        BigTile(Icons.Filled.DarkMode, "Nero", Modifier.weight(1f).fillMaxSize()) { cmd("black") }
    }
    val laser = LocalStatus.current.danger
    val queue = remember { Channel<JSONObject>(Channel.UNLIMITED) }
    LaunchedEffect(id) {
        kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) {
            for (b in queue) runCatching { KonnektaApp.core?.sendPacket(id, Types.PRESENTER, b) }
        }
    }
    Surface(
        shape = RoundedCornerShape(16.dp),
        color = laser.copy(alpha = .10f),
        border = BorderStroke(1.dp, laser.copy(alpha = .4f)),
        modifier = Modifier.fillMaxWidth().weight(1f).padding(vertical = 12.dp).pointerInput(id) {
            detectDragGestures(
                onDragEnd = { queue.trySend(JSONObject().put("stop", true)) },
                onDragCancel = { queue.trySend(JSONObject().put("stop", true)) },
            ) { change, d ->
                change.consume()
                queue.trySend(JSONObject().put("dx", d.x / 1000).put("dy", d.y / 1000))
            }
        },
    ) {
        Box(contentAlignment = Alignment.Center) {
            Text("Trascina qui per il puntatore laser", color = laser)
        }
    }
}

@Composable
private fun BigTile(icon: ImageVector, label: String, modifier: Modifier, primary: Boolean = false, onClick: () -> Unit) {
    val haptic = LocalHapticFeedback.current
    Surface(
        onClick = {
            haptic.performHapticFeedback(HapticFeedbackType.LongPress)
            onClick()
        },
        shape = RoundedCornerShape(18.dp),
        color = if (primary) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.surfaceContainerHigh,
        modifier = modifier,
    ) {
        val fg = if (primary) MaterialTheme.colorScheme.onPrimary else MaterialTheme.colorScheme.onSurface
        Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center) {
            Icon(icon, null, Modifier.size(44.dp), tint = fg)
            Text(label, color = fg)
        }
    }
}

// --- commands ----------------------------------------------------------------------

@Composable
fun CommandsScreen(current: JSONObject?, onBack: () -> Unit) {
    val ui = LocalUi.current
    val scope = rememberCoroutineScope()
    val id = current?.optString("id") ?: return onBack()
    val all by Store.commands.collectAsState()
    LaunchedEffect(id) { send(ui.toast, id, Types.COMMANDS_REQUEST, JSONObject().put("requestCommandList", true)) }
    val cmds = all[id]
    SubHeader("Comandi", current.optString("name"), onBack)
    when {
        cmds == null -> Hint("Chiedo l'elenco al computer…")
        cmds.isEmpty() -> Hint("Nessun comando configurato. Aggiungili sul computer in Comandi.")
    }
    LazyColumn {
        items(cmds?.entries?.toList().orEmpty(), key = { it.key }) { (key, c) ->
            ActionCard(Icons.Filled.PlayCircle, c.optString("name").ifEmpty { key }, c.optString("command")) {
                scope.launch { send(ui.toast, id, Types.COMMANDS_REQUEST, JSONObject().put("key", key)) }
                ui.toast("Avviato: ${c.optString("name")}")
            }
        }
    }
}

// --- Mobile Deck --------------------------------------------------------------------

@Composable
fun DeckScreen(current: JSONObject?, onBack: () -> Unit) {
    val ui = LocalUi.current
    val scope = rememberCoroutineScope()
    val haptic = LocalHapticFeedback.current
    val id = current?.optString("id") ?: return onBack()
    val layouts by Store.deckLayout.collectAsState()
    val allStates by Store.deckStates.collectAsState()
    LaunchedEffect(id) { send(ui.toast, id, Types.DECK_REQUEST, JSONObject().put("request", "layout")) }
    val layout = layouts[id]
    SubHeader("Mobile Deck", current.optString("name"), onBack)
    if (layout == null) {
        Hint("Carico i pulsanti dal computer…")
        return
    }
    val profiles = layout.optJSONArray("profiles")?.objects().orEmpty()
    if (profiles.isEmpty()) {
        Hint("Nessun profilo: crea i pulsanti sul computer in Mobile Deck.")
        return
    }
    var profileId by remember { mutableStateOf(layout.optString("activeProfile")) }
    var page by remember { mutableStateOf(0) }
    var category by remember { mutableStateOf("") }
    val prof = profiles.firstOrNull { it.optString("id") == profileId } ?: profiles.first()
    val pages = prof.optJSONArray("pages")?.objects().orEmpty()
    if (page >= pages.size) page = 0
    val pg = pages.getOrNull(page)
    val cols = prof.optInt("columns", 3).coerceIn(1, 8)
    val rows = prof.optInt("rows", 4).coerceIn(1, 12)
    val buttons = pg?.optJSONArray("buttons")?.objects().orEmpty().associateBy { it.optInt("position") }
    val cats = buttons.values.map { it.optString("category") }.filter { it.isNotEmpty() }.distinct()
    val states = allStates[id] ?: JSONObject()

    LazyRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        if (profiles.size > 1) items(profiles) { p ->
            FilterChip(selected = p.optString("id") == prof.optString("id"), onClick = {
                profileId = p.optString("id")
                page = 0
                scope.launch { send(ui.toast, id, Types.DECK_REQUEST, JSONObject().put("request", "setProfile").put("profile", profileId)) }
            }, label = { Text(p.optString("name")) })
        }
        if (pages.size > 1) items(pages.indices.toList()) { i ->
            FilterChip(selected = i == page, onClick = { page = i }, label = { Text(pages[i].optString("name")) })
        }
        if (cats.size > 1) {
            item { FilterChip(selected = category.isEmpty(), onClick = { category = "" }, label = { Text("Tutti") }) }
            items(cats) { c -> FilterChip(selected = category == c, onClick = { category = c }, label = { Text(c) }) }
        }
    }
    LazyVerticalGrid(GridCells.Fixed(cols), Modifier.fillMaxSize().padding(top = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(10.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        gridItems((0 until cols * rows).toList()) { pos ->
            val b = buttons[pos]
            if (b == null) {
                Spacer(Modifier.aspectRatio(1f))
            } else {
                val st = states.optJSONObject(b.optString("id"))
                val on = st?.optBoolean("active") == true
                val hex = if (on) b.optString("activeColor").ifEmpty { b.optString("color") } else b.optString("color")
                val bg = parseHex(hex) ?: MaterialTheme.colorScheme.primary
                val dim = category.isNotEmpty() && b.optString("category") != category
                Surface(
                    onClick = {
                        haptic.performHapticFeedback(HapticFeedbackType.LongPress)
                        scope.launch {
                            send(ui.toast, id, Types.DECK_REQUEST, JSONObject().put("request", "press")
                                .put("profile", prof.optString("id")).put("page", pg!!.optString("id")).put("button", b.optString("id")))
                        }
                    },
                    shape = RoundedCornerShape(16.dp),
                    color = bg.copy(alpha = if (dim) .25f else 1f),
                    border = if (on) BorderStroke(2.dp, Color.White.copy(alpha = .8f)) else null,
                    modifier = Modifier.aspectRatio(1f),
                ) {
                    Column(Modifier.padding(6.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center) {
                        val icon = b.optString("icon")
                        DECK_ICONS[icon]?.let { Icon(it, null, Modifier.size(30.dp), tint = Color.White) }
                            ?: Text(icon.takeIf { it.length <= 2 } ?: "", fontSize = 24.sp)
                        Text(st?.optString("label")?.ifEmpty { null } ?: b.optString("label"), color = Color.White,
                            fontWeight = FontWeight.SemiBold, fontSize = 12.sp, textAlign = TextAlign.Center,
                            maxLines = 2, overflow = TextOverflow.Ellipsis)
                    }
                }
            }
        }
    }
}

/** Icon names used by the desktop's Deck editor (Material icon ids). */
private val DECK_ICONS: Map<String, ImageVector> = with(Icons.Filled) {
    mapOf(
        "play_pause" to PlayArrow, "play_arrow" to PlayArrow, "skip_previous" to SkipPrevious, "skip_next" to SkipNext,
        "stop" to Stop, "volume" to Icons.AutoMirrored.Filled.VolumeUp, "volume_off" to Icons.AutoMirrored.Filled.VolumeOff,
        "dnd" to DoNotDisturbOn, "content_copy" to ContentCopy, "content_paste" to ContentPaste, "search" to Search,
        "tab" to Tab, "looks_one" to LooksOne, "looks_two" to LooksTwo, "looks_3" to Looks3, "sensors" to Sensors,
        "fiber_manual_record" to FiberManualRecord, "mic_off" to MicOff, "arrow_back" to Icons.AutoMirrored.Filled.ArrowBack,
        "arrow_forward" to Icons.AutoMirrored.Filled.ArrowForward, "dark_mode" to DarkMode, "screenshot" to Screenshot,
        "headset_off" to HeadsetOff, "record_voice_over" to RecordVoiceOver, "work" to Work, "videocam" to Videocam,
        "slideshow" to Slideshow, "sports_esports" to SportsEsports, "lock" to Lock, "terminal" to Terminal,
    )
}

private fun parseHex(hex: String): Color? =
    if (hex.length == 7 && hex.startsWith("#")) hex.substring(1).toLongOrNull(16)?.let { Color(0xFF000000 or it) } else null
