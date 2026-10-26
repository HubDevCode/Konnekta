package org.konnekta.app.ui

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.media.projection.MediaProjectionManager
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.withContext
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.core.Types
import org.konnekta.app.service.KonnektaService
import org.konnekta.app.service.LiveStream
import org.konnekta.app.service.STREAM_ASK
import org.konnekta.app.service.STREAM_NAMES
import org.konnekta.app.service.StreamRequest
import org.konnekta.app.service.Streams
import org.konnekta.app.state.Settings
import org.konnekta.app.state.Store
import kotlin.math.abs

/**
 * Starts a stream the phone produces: Android permissions first, then the
 * screen-capture consent when needed, then the service. Used for the
 * computer's requests and for the buttons in Control.
 */
@Composable
fun rememberStreamStarter(): (StreamRequest) -> Unit {
    val ctx = LocalContext.current
    val ui = LocalUi.current
    var waiting by remember { mutableStateOf<StreamRequest?>(null) }
    val projection = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { res ->
        val r = waiting ?: return@rememberLauncherForActivityResult
        waiting = null
        if (res.resultCode == Activity.RESULT_OK && res.data != null) KonnektaService.acceptStream(ctx, r, res.data, res.resultCode)
        else Streams.decline(r)
    }
    val permissions = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { granted ->
        val r = waiting ?: return@rememberLauncherForActivityResult
        if (granted.values.all { it }) {
            if (r.plugin in Streams.NEEDS_PROJECTION) {
                projection.launch(ctx.getSystemService(MediaProjectionManager::class.java).createScreenCaptureIntent())
            } else {
                waiting = null
                KonnektaService.acceptStream(ctx, r)
            }
        } else {
            waiting = null
            ui.toast("Permesso negato")
            Streams.decline(r)
        }
    }
    return { r ->
        val needed = buildList {
            if (r.plugin == "camera") add(Manifest.permission.CAMERA)
            if (r.plugin in setOf("microphone", "phoneaudio") || r.params.optBoolean("withAudio")) add(Manifest.permission.RECORD_AUDIO)
        }.toTypedArray()
        waiting = r
        // The permission step always runs (instantly when already granted) so both paths share one flow.
        permissions.launch(needed)
    }
}

/** "omarchy asks for the webcam": start it (with the Android permission) or not. */
@Composable
fun StreamRequestTile(r: StreamRequest) {
    val start = rememberStreamStarter()
    Surface(
        shape = RoundedCornerShape(16.dp),
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        border = BorderStroke(1.5.dp, MaterialTheme.colorScheme.primary),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Pill(STREAM_NAMES[r.plugin]?.replaceFirstChar { it.uppercase() } ?: r.plugin, MaterialTheme.colorScheme.secondary)
            Text("${r.deviceName} chiede ${STREAM_ASK[r.plugin]}", style = MaterialTheme.typography.titleLarge)
            Text(
                when (r.plugin) {
                    "microphone" -> "Il computer userà il telefono come microfono finché non lo fermi."
                    "camera" -> "Il computer vedrà la fotocamera come una webcam finché non la fermi."
                    "phoneaudio" -> "Il computer riprodurrà l'audio del telefono (musica, video, giochi) finché non lo fermi."
                    else -> "Il computer vedrà tutto ciò che appare sullo schermo del telefono."
                },
                style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End)) {
                OutlinedButton(onClick = { Streams.decline(r) }) { Text("Non ora") }
                Button(onClick = { start(r) }) {
                    Text(when (r.plugin) {
                        "camera" -> "Avvia webcam"
                        "microphone" -> "Avvia microfono"
                        "phoneaudio" -> "Condividi audio"
                        else -> "Condividi schermo"
                    })
                }
            }
        }
    }
}

/** A stream running now, with Stop (as the computer's live features). */
@Composable
fun LiveStreamCard(l: LiveStream) {
    val danger = LocalStatus.current.danger
    Surface(
        shape = RoundedCornerShape(14.dp),
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        border = BorderStroke(1.dp, danger.copy(alpha = .5f)),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(Modifier.padding(start = 14.dp, top = 10.dp, bottom = 10.dp, end = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Pill("Dal vivo", danger)
                Text(
                    (STREAM_NAMES[l.plugin] ?: l.plugin).replaceFirstChar { it.uppercase() } +
                        if (l.outgoing) " → ${Store.name(l.deviceId)}" else " da ${Store.name(l.deviceId)}",
                    style = MaterialTheme.typography.titleSmall,
                )
            }
            OutlinedButton(onClick = { Thread { Streams.stop(l.deviceId, l.plugin) }.start() }) { Text("Ferma", color = danger) }
        }
    }
}

/**
 * The computer's screen. Tap = click where you tap, drag = move the
 * pointer, two fingers = scroll, long press = right click.
 */
@Composable
fun ColumnScope.RemoteDesktopScreen(current: JSONObject?, onBack: () -> Unit) {
    val id = current?.optString("id") ?: return onBack()
    val ctx = LocalContext.current
    val live by Streams.live.collectAsState()
    val size by Streams.desktopSize.collectAsState()
    val running = live.containsKey("desktop/$id")
    val pointerMode by Settings.pointerMode.collectAsState()
    val orientation by Settings.desktopOrientation.collectAsState()
    val queue = remember { Channel<JSONObject>(Channel.UNLIMITED) }
    // Horizontal turns the screen, vertical keeps it upright (Impostazioni).
    LockOrientation(orientation)
    // Ask (again) whenever the computer is connected and nothing is running.
    val connected = current.optBoolean("connected")
    LaunchedEffect(id, connected) {
        val m = ctx.resources.displayMetrics
        if (connected && !Streams.live.value.containsKey("desktop/$id")) {
            Streams.request(id, "desktop", JSONObject().put("width", maxOf(m.widthPixels, m.heightPixels)).put("height", minOf(m.widthPixels, m.heightPixels)).put("fps", 60))
        }
    }
    // The computer's sound too, when chosen in Impostazioni.
    ScreenAudio(id, connected, if (Settings.desktopAudio.value) Settings.ScreenAudio.Duplicate else Settings.ScreenAudio.None)
    LaunchedEffect(id) { sendInput(id, queue) }
    DisposableEffect(id) { onDispose { Thread { Streams.stop(id, "desktop") }.start() } }

    val ratio = size?.let { (w, h) -> w.toFloat() / h } ?: (16f / 9f)
    val video: @Composable (Modifier) -> Unit = { m ->
        Box(m.aspectRatio(ratio).background(Color.Black)) {
            AndroidView(factory = { c ->
                SurfaceView(c).apply {
                    holder.addCallback(object : SurfaceHolder.Callback {
                        override fun surfaceCreated(h: SurfaceHolder) { Streams.desktopSurface = h.surface }
                        override fun surfaceChanged(h: SurfaceHolder, f: Int, w: Int, hh: Int) {}
                        override fun surfaceDestroyed(h: SurfaceHolder) { Streams.desktopSurface = null }
                    })
                }
            }, Modifier.fillMaxSize())
            Box(Modifier.fillMaxSize().pointerInput(id, pointerMode) { remoteTouch(queue, pointerMode, extend = false) })
            if (size == null) Text(if (running) "Attendo l'immagine…" else "Chiedo lo schermo al computer…",
                Modifier.align(Alignment.Center), color = Color.White)
        }
    }
    androidx.compose.foundation.layout.BoxWithConstraints(Modifier.fillMaxWidth().weight(1f)) {
        if (maxWidth > maxHeight) {
            // Landscape: the computer's screen as large as it fits, only a small back button on top.
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                video(Modifier.fillMaxHeight())
                Surface(onClick = onBack, shape = RoundedCornerShape(12.dp), color = Color.Black.copy(alpha = .45f),
                    modifier = Modifier.align(Alignment.TopStart).padding(8.dp)) {
                    androidx.compose.material3.Icon(Icons.AutoMirrored.Filled.ArrowBack, "Indietro",
                        Modifier.padding(8.dp), tint = Color.White)
                }
            }
        } else {
            Column(Modifier.fillMaxSize()) {
                SubHeader("Desktop remoto", current.optString("name"), onBack)
                Box(Modifier.fillMaxWidth().weight(1f), contentAlignment = Alignment.Center) { video(Modifier.fillMaxWidth()) }
                Hint(pointerHint(pointerMode) + if (orientation == Settings.ScreenOrientation.Auto) " Ruota il telefono per vederlo più grande." else "")
            }
        }
    }
}

/**
 * The computer's sound while a screen is shown: what it plays
 * ([Settings.ScreenAudio.Duplicate]), its own output for the second screen
 * (Separate), or nothing. Stopped with the screen, unless it was already
 * on from Controllo.
 */
@Composable
private fun ScreenAudio(id: String, connected: Boolean, mode: Settings.ScreenAudio) {
    var started by remember { mutableStateOf(false) }
    LaunchedEffect(id, connected, mode) {
        if (!connected || mode == Settings.ScreenAudio.None || Streams.live.value.containsKey("systemaudio/$id")) return@LaunchedEffect
        val params = JSONObject().put("codec", "pcm_s16le").put("sampleRate", 48000).put("channels", 2)
        if (mode == Settings.ScreenAudio.Separate) params.put("source", "screen")
        Streams.request(id, "systemaudio", params)
        started = true
    }
    DisposableEffect(id) {
        onDispose { if (started) Thread { Streams.stop(id, "systemaudio") }.start() }
    }
}

/** Hold the activity in [o] while on screen; Auto follows the phone's rotation. */
@Composable
private fun LockOrientation(o: Settings.ScreenOrientation) {
    val ctx = LocalContext.current
    DisposableEffect(o) {
        val act = ctx as? Activity
        val before = act?.requestedOrientation
        when (o) {
            Settings.ScreenOrientation.Landscape -> act?.requestedOrientation = android.content.pm.ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
            Settings.ScreenOrientation.Portrait -> act?.requestedOrientation = android.content.pm.ActivityInfo.SCREEN_ORIENTATION_SENSOR_PORTRAIT
            Settings.ScreenOrientation.Auto -> {}
        }
        onDispose { if (before != null && o != Settings.ScreenOrientation.Auto) act.requestedOrientation = before }
    }
}

/** How to use the touch surface, for the current pointer mode. */
fun pointerHint(mode: Settings.PointerMode) = when (mode) {
    Settings.PointerMode.Direct -> "Tocca per cliccare dove tocchi · trascina per trascinare · due dita per scorrere · tieni premuto per il tasto destro · tre dita di lato cambiano workspace."
    Settings.PointerMode.Touchpad -> "Trascina per muovere il puntatore · tocca per cliccare · tocca due volte e trascina per selezionare · due dita per scorrere · tieni premuto per il tasto destro."
}

/**
 * Touches on the computer's picture. [mode]: the pointer goes where the
 * finger is (absolute), or the picture is a touchpad (relative moves, at a
 * speed that crosses the computer's screen as the finger crosses the
 * picture). [extend]: the picture is the phone's own virtual monitor.
 */
private suspend fun androidx.compose.ui.input.pointer.PointerInputScope.remoteTouch(
    queue: Channel<JSONObject>, mode: Settings.PointerMode, extend: Boolean,
) {
    fun abs(x: Float, y: Float) = JSONObject()
        .put("konnektaAbsX", (x / size.width).coerceIn(0f, 1f).toDouble())
        .put("konnektaAbsY", (y / size.height).coerceIn(0f, 1f).toDouble())
        .apply { if (extend) put("konnektaScreen", "extend") }
    val touchpad = mode == Settings.PointerMode.Touchpad
    val swipeMin = 45.dp.toPx()
    var lastTap = 0L
    awaitEachGesture {
        val down = awaitFirstDown()
        val start = down.uptimeMillis
        // Touchpad: a tap and then a press right after drags (button held).
        val dragHold = touchpad && start - lastTap < 300
        var holding = false
        var maxPointers = 1
        var moved = 0f
        var dragging = false
        val swipe = ThreeFingerSwipe(swipeMin)
        // Touchpad speed: the whole picture is the whole computer screen.
        val remote = Streams.desktopSize.value
        val gain = remote?.let { (rw, _) -> (rw.toFloat() / size.width).coerceIn(0.5f, 6f) } ?: 1.5f
        while (true) {
            val ev = awaitPointerEvent()
            val pressed = ev.changes.filter { it.pressed }
            maxPointers = maxOf(maxPointers, pressed.size)
            if (pressed.isEmpty()) break
            if (ev.type == PointerEventType.Move) {
                val c = pressed.first()
                val dx = c.position.x - c.previousPosition.x
                val dy = c.position.y - c.previousPosition.y
                moved += abs(dx) + abs(dy)
                ev.changes.forEach { it.consume() }
                if (maxPointers >= 3) {
                    // Three fingers: switch workspace (left/right) or overview (up/down).
                    swipe.add(pressed)?.let { queue.trySend(JSONObject().put("konnektaSwipe", it)) }
                } else if (pressed.size >= 2) {
                    // Raw distance: the computer turns it into wheel steps and keeps the remainder.
                    val k = Settings.scrollSpeed.value
                    queue.trySend(JSONObject().put("dx", dx * k).put("dy", dy * k).put("scroll", true))
                } else if (touchpad) {
                    if (dragHold && !holding && moved > 6f) {
                        holding = true
                        queue.trySend(JSONObject().put("singlehold", true))
                    }
                    if (moved > 3f) {
                        dragging = true
                        queue.trySend(JSONObject().put("dx", (dx * gain).toDouble()).put("dy", (dy * gain).toDouble()))
                    }
                } else if (moved > 12f) {
                    // Direct: the first move presses the button where the finger
                    // started, so dragging selects or moves windows.
                    if (!dragging) {
                        dragging = true
                        queue.trySend(abs(down.position.x, down.position.y).put("singlehold", true))
                        holding = true
                    }
                    queue.trySend(abs(c.position.x, c.position.y))
                }
            }
        }
        val held = android.os.SystemClock.uptimeMillis() - start
        if (holding) queue.trySend(JSONObject().put("singlerelease", true))
        when {
            dragging || maxPointers >= 2 -> {}
            touchpad && moved < 12f && isLongPress(held) -> queue.trySend(JSONObject().put("rightclick", true))
            touchpad && moved < 12f -> {
                queue.trySend(JSONObject().put("singleclick", true))
                lastTap = android.os.SystemClock.uptimeMillis()
            }
            moved < 12f && isLongPress(held) -> queue.trySend(abs(down.position.x, down.position.y).put("rightclick", true))
            moved < 12f -> queue.trySend(abs(down.position.x, down.position.y).put("singleclick", true))
        }
    }
}

/**
 * The phone as a second monitor of the computer: the computer makes a
 * monitor of the phone's size (arranged in its display settings like any
 * other) and the phone shows it full screen, in landscape or, as set in
 * Impostazioni, in portrait (then the monitor is portrait too). Touches
 * drive the pointer as set in Impostazioni (where you touch, or touchpad).
 */
@Composable
fun ExtendScreen(current: JSONObject?, onBack: () -> Unit) {
    val id = current?.optString("id") ?: return onBack()
    val ctx = LocalContext.current
    val live by Streams.live.collectAsState()
    val size by Streams.desktopSize.collectAsState()
    val running = live.containsKey("extend/$id")
    val pointerMode by Settings.pointerMode.collectAsState()
    val queue = remember { Channel<JSONObject>(Channel.UNLIMITED) }
    var help by remember { mutableStateOf(true) }
    val connected = current.optBoolean("connected")
    // Read once: the monitor's shape is fixed for the session.
    val portrait = remember { Settings.extendOrientation.value == Settings.ScreenOrientation.Portrait }
    // Landscape or portrait, full screen, screen kept on while it shows the monitor.
    DisposableEffect(Unit) {
        val act = ctx as? Activity
        val before = act?.requestedOrientation
        act?.requestedOrientation = if (portrait) android.content.pm.ActivityInfo.SCREEN_ORIENTATION_SENSOR_PORTRAIT
            else android.content.pm.ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
        act?.window?.addFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val bars = act?.window?.let { androidx.core.view.WindowCompat.getInsetsController(it, it.decorView) }
        bars?.systemBarsBehavior = androidx.core.view.WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        bars?.hide(androidx.core.view.WindowInsetsCompat.Type.systemBars())
        onDispose {
            bars?.show(androidx.core.view.WindowInsetsCompat.Type.systemBars())
            act?.window?.clearFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            if (before != null) act.requestedOrientation = before
        }
    }
    LaunchedEffect(id, connected) {
        if (connected && !Streams.live.value.containsKey("extend/$id")) {
            // The real size of the screen, in the chosen orientation: the monitor matches it pixel for pixel.
            val b = if (android.os.Build.VERSION.SDK_INT >= 30) (ctx as? Activity)?.windowManager?.maximumWindowMetrics?.bounds else null
            val (long, short) = b?.let { maxOf(it.width(), it.height()) to minOf(it.width(), it.height()) }
                ?: ctx.resources.displayMetrics.let { maxOf(it.widthPixels, it.heightPixels) to minOf(it.widthPixels, it.heightPixels) }
            val (w, h) = if (portrait) short to long else long to short
            Streams.request(id, "extend", JSONObject().put("width", w).put("height", h).put("fps", 60))
        }
    }
    ScreenAudio(id, connected, remember { Settings.extendAudio.value })
    LaunchedEffect(id) { sendInput(id, queue) }
    LaunchedEffect(help) {
        if (help) {
            kotlinx.coroutines.delay(6000)
            help = false
        }
    }
    DisposableEffect(id) { onDispose { Thread { Streams.stop(id, "extend") }.start() } }
    BackHandler { onBack() }

    Box(Modifier.fillMaxSize().background(Color.Black), contentAlignment = Alignment.Center) {
        val ratio = size?.let { (w, h) -> w.toFloat() / h } ?: (16f / 9f)
        Box(Modifier.fillMaxSize().aspectRatio(ratio, matchHeightConstraintsFirst = true)) {
            AndroidView(factory = { c ->
                SurfaceView(c).apply {
                    holder.addCallback(object : SurfaceHolder.Callback {
                        override fun surfaceCreated(h: SurfaceHolder) { Streams.desktopSurface = h.surface }
                        override fun surfaceChanged(h: SurfaceHolder, f: Int, w: Int, hh: Int) {}
                        override fun surfaceDestroyed(h: SurfaceHolder) { Streams.desktopSurface = null }
                    })
                }
            }, Modifier.fillMaxSize())
            Box(Modifier.fillMaxSize().pointerInput(id, pointerMode) { remoteTouch(queue, pointerMode, extend = true) })
        }
        if (size == null) Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Text(if (running) "Il monitor è pronto, attendo l'immagine…" else "Chiedo a ${current.optString("name")} un nuovo monitor…", color = Color.White)
            Text("Su ${current.optString("name")} il telefono compare come monitor nelle impostazioni dello schermo.",
                color = Color.White.copy(alpha = .7f), style = MaterialTheme.typography.bodySmall)
        }
        if (help && size != null) Surface(color = Color.Black.copy(alpha = .6f), shape = RoundedCornerShape(12.dp),
            modifier = Modifier.align(Alignment.BottomCenter).padding(12.dp)) {
            Text(pointerHint(pointerMode) + " Indietro per chiudere.", Modifier.padding(10.dp), color = Color.White,
                style = MaterialTheme.typography.bodySmall)
        }
        Surface(onClick = onBack, shape = RoundedCornerShape(12.dp), color = Color.Black.copy(alpha = .35f),
            modifier = Modifier.align(Alignment.TopStart).padding(8.dp)) {
            androidx.compose.material3.Icon(Icons.AutoMirrored.Filled.ArrowBack, "Chiudi lo schermo secondario",
                Modifier.padding(6.dp), tint = Color.White.copy(alpha = .8f))
        }
    }
}

/** A press this long is a right click. */
private fun isLongPress(heldMs: Long) = heldMs in 550..5000
