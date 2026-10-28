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
import androidx.compose.foundation.layout.BoxScope
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
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.layout.layout
import androidx.compose.ui.unit.Constraints
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
import kotlin.math.roundToInt

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
    val zoomOn by Settings.desktopZoom.collectAsState()
    val zoomState = remember { Zoom() }
    val zoom = zoomState.takeIf { zoomOn }
    LaunchedEffect(zoomState) { Streams.desktopCursor.collect { c -> c?.let { (x, y) -> zoomState.onCursor(x, y) } } }
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
        Box(m.aspectRatio(ratio).background(Color.Black).clipToBounds()) {
            AndroidView(factory = { c ->
                SurfaceView(c).apply {
                    holder.addCallback(object : SurfaceHolder.Callback {
                        override fun surfaceCreated(h: SurfaceHolder) { Streams.desktopSurface = h.surface }
                        override fun surfaceChanged(h: SurfaceHolder, f: Int, w: Int, hh: Int) {}
                        override fun surfaceDestroyed(h: SurfaceHolder) { Streams.desktopSurface = null }
                    })
                }
            }, Modifier.zoomed(zoom))
            Box(Modifier.fillMaxSize().pointerInput(id, pointerMode, zoom) { remoteTouch(queue, pointerMode, extend = false, zoom) })
            zoom?.let { ZoomReset(it) }
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
                Hint(pointerHint(pointerMode, zoomOn) + if (orientation == Settings.ScreenOrientation.Auto) " Ruota il telefono per vederlo più grande." else "")
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

/** How to use the touch surface, for the current pointer mode (and pinch zoom, when on). */
fun pointerHint(mode: Settings.PointerMode, zoom: Boolean = false) = when (mode) {
    Settings.PointerMode.Direct -> "Tocca per cliccare dove tocchi · trascina per trascinare · due dita per scorrere · tieni premuto per il tasto destro · tre dita di lato cambiano workspace."
    Settings.PointerMode.Touchpad -> "Trascina per muovere il puntatore · tocca per cliccare · tocca due volte e trascina per selezionare · due dita per scorrere · tieni premuto per il tasto destro."
} + if (zoom) " Allarga due dita per ingrandire: la vista segue il puntatore." else ""

/**
 * Pinch zoom of the computer's picture: [zoom] times bigger, centred on
 * ([cx], [cy]) as shares of the picture. ([px], [py]) is where the
 * computer's pointer is, so the view can follow it: the more zoomed in, the
 * closer it follows. The computer tells where it is ([tracked], see
 * [onCursor]); else the phone places it itself and knows from that.
 */
class Zoom {
    var zoom by mutableFloatStateOf(1f)
    var cx by mutableFloatStateOf(.5f)
    var cy by mutableFloatStateOf(.5f)
    var px = .5f
    var py = .5f

    /** The computer reports its pointer. */
    var tracked = false
        private set

    /**
     * Fingers place the pointer where they touch: the view stays still under
     * them and only moves when the pointer leaves it (moved from the computer).
     */
    var direct = false

    /** A finger is on the picture in [direct] mode. */
    var hold = false

    /** The computer's pointer is at ([x], [y]): follow it while it is on this screen. */
    fun onCursor(x: Float, y: Float) {
        tracked = true
        px = x
        py = y
        if (!hold && x in 0f..1f && y in 0f..1f) follow(margin = !direct)
    }

    val left get() = cx - .5f / zoom
    val top get() = cy - .5f / zoom

    /** Share of the picture shown at the share ([u], [v]) of the view. */
    fun toPicture(u: Float, v: Float) = (left + u / zoom) to (top + v / zoom)

    fun reset() {
        zoom = 1f
        cx = .5f
        cy = .5f
    }

    private fun clamp() {
        val h = .5f / zoom
        cx = cx.coerceIn(h, 1 - h)
        cy = cy.coerceIn(h, 1 - h)
    }

    /**
     * Two fingers apart/together: scale by [k], keeping the picture that was
     * under the fingers' middle ([u0], [v0]) under its new place ([u1], [v1]),
     * so moving both fingers also pans. Shares of the view.
     */
    fun pinch(k: Float, u0: Float, v0: Float, u1: Float, v1: Float) {
        val (pu, pv) = toPicture(u0, v0)
        zoom = (zoom * k).coerceIn(1f, MAX)
        cx = pu - u1 / zoom + .5f / zoom
        cy = pv - v1 / zoom + .5f / zoom
        clamp()
    }

    /**
     * Touchpad: move the pointer by ([du], [dv]) shares of the view (finer
     * when zoomed in). If it was off the view (after a pinch moved away) it
     * starts again from the middle of what is shown.
     */
    fun movePointer(du: Float, dv: Float) {
        if (px !in left..left + 1 / zoom || py !in top..top + 1 / zoom) {
            px = cx
            py = cy
        }
        px = (px + du / zoom).coerceIn(0f, 1f)
        py = (py + dv / zoom).coerceIn(0f, 1f)
        follow()
    }

    /**
     * Keep the pointer inside the view, away from its edges by a [margin]
     * that grows with the zoom: barely zoomed it can reach the edge, fully
     * zoomed it stays near the middle.
     */
    private fun follow(margin: Boolean = true) {
        if (zoom <= 1f) return
        val m = if (!margin) 0f else (.1f + .25f * (zoom - 1) / (MAX - 1)) / zoom
        val h = .5f / zoom
        cx = cx.coerceIn(px - h + m, px + h - m)
        cy = cy.coerceIn(py - h + m, py + h - m)
        clamp()
    }

    /**
     * "Where you touch", dragging with the finger near an edge of the view:
     * slide the view that way, faster closer to the edge. [dt] in seconds.
     * Returns whether the view moved.
     */
    fun edgePan(u: Float, v: Float, dt: Float): Boolean {
        if (zoom <= 1f) return false
        fun push(s: Float) = when {
            s < EDGE -> (s - EDGE) / EDGE
            s > 1 - EDGE -> (s - 1 + EDGE) / EDGE
            else -> 0f
        }
        val (ox, oy) = cx to cy
        cx += push(u) * PAN_SPEED * dt / zoom
        cy += push(v) * PAN_SPEED * dt / zoom
        clamp()
        return cx != ox || cy != oy
    }

    companion object {
        const val MAX = 5f
        /** Edge band (share of the view) that pans while dragging. */
        private const val EDGE = .12f
        /** View widths per second at the very edge. */
        private const val PAN_SPEED = .8f
    }
}

/** The picture [z] times bigger, shifted so the zoomed part fills the box (which clips it). */
private fun Modifier.zoomed(z: Zoom?) = if (z == null) fillMaxSize() else layout { m, c ->
    val w = c.maxWidth
    val h = c.maxHeight
    val p = m.measure(Constraints.fixed((w * z.zoom).roundToInt(), (h * z.zoom).roundToInt()))
    layout(w, h) { p.place((-z.left * p.width).roundToInt(), (-z.top * p.height).roundToInt()) }
}

/** While zoomed in: how much, and a tap back to 1×. */
@Composable
private fun BoxScope.ZoomReset(z: Zoom) {
    if (z.zoom > 1.01f) Surface(onClick = { z.reset() }, shape = RoundedCornerShape(12.dp), color = Color.Black.copy(alpha = .45f),
        modifier = Modifier.align(Alignment.TopEnd).padding(8.dp)) {
        Text("${"%.1f".format(z.zoom)}× · torna a 1×", Modifier.padding(horizontal = 10.dp, vertical = 6.dp), color = Color.White,
            style = MaterialTheme.typography.labelMedium)
    }
}

/**
 * Touches on the computer's picture. [mode]: the pointer goes where the
 * finger is (absolute), or the picture is a touchpad (relative moves, at a
 * speed that crosses the computer's screen as the finger crosses the
 * picture). [extend]: the picture is the phone's own virtual monitor.
 * [zoom]: two fingers apart/together zoom the picture (else they only scroll).
 */
private suspend fun androidx.compose.ui.input.pointer.PointerInputScope.remoteTouch(
    queue: Channel<JSONObject>, mode: Settings.PointerMode, extend: Boolean, zoom: Zoom?,
) {
    fun absShare(u: Float, v: Float) = JSONObject()
        .put("konnektaAbsX", u.coerceIn(0f, 1f).toDouble())
        .put("konnektaAbsY", v.coerceIn(0f, 1f).toDouble())
        .apply { if (extend) put("konnektaScreen", "extend") }
    // The computer's screen under (x, y) on the view, through the zoom.
    fun abs(x: Float, y: Float): JSONObject {
        val u = (x / size.width).coerceIn(0f, 1f)
        val v = (y / size.height).coerceIn(0f, 1f)
        val (pu, pv) = zoom?.toPicture(u, v) ?: (u to v)
        zoom?.apply { px = pu; py = pv }
        return absShare(pu, pv)
    }
    val touchpad = mode == Settings.PointerMode.Touchpad
    val swipeMin = 45.dp.toPx()
    val pinchMin = 24.dp.toPx()
    val scrollMin = 12.dp.toPx()
    var lastTap = 0L
    zoom?.direct = !touchpad
    awaitEachGesture {
        val down = awaitFirstDown()
        zoom?.hold = !touchpad
        val start = down.uptimeMillis
        // Touchpad: a tap and then a press right after drags (button held).
        val dragHold = touchpad && start - lastTap < 300
        var holding = false
        var maxPointers = 1
        var moved = 0f
        var dragging = false
        val swipe = ThreeFingerSwipe(swipeMin)
        // Two fingers with zoom on: pinching (true) or scrolling (false), once it is clear which.
        var pinching: Boolean? = null
        var span0 = -1f
        var mid0 = Offset.Zero
        var last = down.position
        var lastTick = down.uptimeMillis
        // Touchpad speed: the whole picture is the whole computer screen.
        val remote = Streams.desktopSize.value
        val gain = remote?.let { (rw, _) -> (rw.toFloat() / size.width).coerceIn(0.5f, 6f) } ?: 1.5f
        while (true) {
            // Dragging "where you touch" while zoomed: tick so a finger held at an edge keeps panning.
            val edgeDrag = zoom != null && !touchpad && dragging && maxPointers == 1 && zoom.zoom > 1f
            val ev = if (edgeDrag) withTimeoutOrNull(16) { awaitPointerEvent() } else awaitPointerEvent()
            val now = android.os.SystemClock.uptimeMillis()
            if (edgeDrag && zoom.edgePan(last.x / size.width, last.y / size.height, (now - lastTick) / 1000f)) {
                queue.trySend(abs(last.x, last.y))
            }
            lastTick = now
            if (ev == null) continue
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
                    if (zoom != null && pinching != false) {
                        val (a, b) = pressed
                        val span = (a.position - b.position).getDistance()
                        val prevSpan = (a.previousPosition - b.previousPosition).getDistance()
                        val mid = (a.position + b.position) / 2f
                        val prevMid = (a.previousPosition + b.previousPosition) / 2f
                        if (pinching == null) {
                            // Fingers apart/together first: zoom; moving together first: scroll.
                            if (span0 < 0f) {
                                span0 = prevSpan
                                mid0 = prevMid
                            }
                            if (abs(span - span0) > pinchMin) pinching = true
                            else if ((mid - mid0).getDistance() > scrollMin) pinching = false
                        }
                        if (pinching == true && prevSpan > 0f) zoom.pinch(
                            span / prevSpan, prevMid.x / size.width, prevMid.y / size.height, mid.x / size.width, mid.y / size.height,
                        )
                    }
                    if (zoom == null || pinching == false) {
                        // Raw distance: the computer turns it into wheel steps and keeps the remainder.
                        val k = Settings.scrollSpeed.value
                        queue.trySend(JSONObject().put("dx", dx * k).put("dy", dy * k).put("scroll", true))
                    }
                } else if (maxPointers >= 2) {
                    // One finger left after scrolling or zooming: not a pointer move.
                } else if (touchpad) {
                    if (dragHold && !holding && moved > 6f) {
                        holding = true
                        queue.trySend(JSONObject().put("singlehold", true))
                    }
                    if (moved > 3f) {
                        dragging = true
                        if (zoom != null && zoom.zoom > 1f && !zoom.tracked) {
                            // The computer does not tell where its pointer is: the phone places
                            // it, so it knows where it is and the view can follow it.
                            zoom.movePointer(dx / size.width, dy / size.height)
                            queue.trySend(absShare(zoom.px, zoom.py))
                        } else {
                            // Zoomed in, finer: crossing the view crosses what it shows.
                            val g = gain / (zoom?.zoom ?: 1f)
                            queue.trySend(JSONObject().put("dx", (dx * g).toDouble()).put("dy", (dy * g).toDouble()))
                        }
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
                    last = c.position
                }
            }
        }
        zoom?.hold = false
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
    val zoomOn by Settings.extendZoom.collectAsState()
    val zoomState = remember { Zoom() }
    val zoom = zoomState.takeIf { zoomOn }
    LaunchedEffect(zoomState) { Streams.desktopCursor.collect { c -> c?.let { (x, y) -> zoomState.onCursor(x, y) } } }
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
            val (w, h) = extendSize(ctx, portrait, Settings.extendScale.value)
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
        Box(Modifier.fillMaxSize().aspectRatio(ratio, matchHeightConstraintsFirst = true).clipToBounds()) {
            AndroidView(factory = { c ->
                SurfaceView(c).apply {
                    holder.addCallback(object : SurfaceHolder.Callback {
                        override fun surfaceCreated(h: SurfaceHolder) { Streams.desktopSurface = h.surface }
                        override fun surfaceChanged(h: SurfaceHolder, f: Int, w: Int, hh: Int) {}
                        override fun surfaceDestroyed(h: SurfaceHolder) { Streams.desktopSurface = null }
                    })
                }
            }, Modifier.zoomed(zoom))
            Box(Modifier.fillMaxSize().pointerInput(id, pointerMode, zoom) { remoteTouch(queue, pointerMode, extend = true, zoom) })
            zoom?.let { ZoomReset(it) }
        }
        if (size == null) Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Text(if (running) "Il monitor è pronto, attendo l'immagine…" else "Chiedo a ${current.optString("name")} un nuovo monitor…", color = Color.White)
            Text("Su ${current.optString("name")} il telefono compare come monitor nelle impostazioni dello schermo.",
                color = Color.White.copy(alpha = .7f), style = MaterialTheme.typography.bodySmall)
        }
        if (help && size != null) Surface(color = Color.Black.copy(alpha = .6f), shape = RoundedCornerShape(12.dp),
            modifier = Modifier.align(Alignment.BottomCenter).padding(12.dp)) {
            Text(pointerHint(pointerMode, zoomOn) + " Indietro per chiudere.", Modifier.padding(10.dp), color = Color.White,
                style = MaterialTheme.typography.bodySmall)
        }
        Surface(onClick = onBack, shape = RoundedCornerShape(12.dp), color = Color.Black.copy(alpha = .35f),
            modifier = Modifier.align(Alignment.TopStart).padding(8.dp)) {
            androidx.compose.material3.Icon(Icons.AutoMirrored.Filled.ArrowBack, "Chiudi lo schermo secondario",
                Modifier.padding(6.dp), tint = Color.White.copy(alpha = .8f))
        }
    }
}

/**
 * Size of the second screen's monitor: the phone's real screen in the chosen
 * orientation, times [scale] (same shape, even numbers). At 1 it matches the
 * phone pixel for pixel; lower shows everything bigger.
 */
fun extendSize(ctx: android.content.Context, portrait: Boolean, scale: Float): Pair<Int, Int> {
    val b = if (android.os.Build.VERSION.SDK_INT >= 30) (ctx as? Activity)?.windowManager?.maximumWindowMetrics?.bounds else null
    val (long, short) = b?.let { maxOf(it.width(), it.height()) to minOf(it.width(), it.height()) }
        ?: ctx.resources.displayMetrics.let { maxOf(it.widthPixels, it.heightPixels) to minOf(it.widthPixels, it.heightPixels) }
    val (w, h) = if (portrait) short to long else long to short
    return ((w * scale).toInt() and 1.inv()) to ((h * scale).toInt() and 1.inv())
}

/** A press this long is a right click. */
private fun isLongPress(heldMs: Long) = heldMs in 550..5000
