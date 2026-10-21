package org.konnekta.app.ui

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.media.projection.MediaProjectionManager
import android.view.SurfaceHolder
import android.view.SurfaceView
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
    val queue = remember { Channel<JSONObject>(Channel.UNLIMITED) }
    // Ask (again) whenever the computer is connected and nothing is running.
    val connected = current.optBoolean("connected")
    LaunchedEffect(id, connected) {
        val m = ctx.resources.displayMetrics
        if (connected && !Streams.live.value.containsKey("desktop/$id")) {
            Streams.request(id, "desktop", JSONObject().put("width", maxOf(m.widthPixels, m.heightPixels)).put("height", minOf(m.widthPixels, m.heightPixels)).put("fps", 30))
        }
    }
    LaunchedEffect(id) {
        withContext(Dispatchers.IO) {
            for (b in queue) runCatching { KonnektaApp.core?.sendPacket(id, Types.INPUT, b) }
        }
    }
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
            Box(Modifier.fillMaxSize().pointerInput(id) { remoteTouch(queue) })
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
                Hint("Tocca per cliccare · trascina per muovere · due dita per scorrere · tre dita di lato cambiano workspace · tieni premuto per il tasto destro. " +
                    "Ruota il telefono per vederlo più grande.")
            }
        }
    }
}

private suspend fun androidx.compose.ui.input.pointer.PointerInputScope.remoteTouch(queue: Channel<JSONObject>) {
    fun abs(x: Float, y: Float) = JSONObject()
        .put("konnektaAbsX", (x / size.width).coerceIn(0f, 1f).toDouble())
        .put("konnektaAbsY", (y / size.height).coerceIn(0f, 1f).toDouble())
    val swipeMin = 60.dp.toPx()
    awaitEachGesture {
        val down = awaitFirstDown()
        val start = down.uptimeMillis
        var maxPointers = 1
        var moved = 0f
        var dragging = false
        var swipeX = 0f
        var swipeY = 0f
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
                    swipeX += dx
                    swipeY += dy
                } else if (pressed.size >= 2) {
                    // Raw distance: the computer turns it into wheel steps and keeps the remainder.
                    val k = Settings.scrollSpeed.value
                    queue.trySend(JSONObject().put("dx", dx * k).put("dy", dy * k).put("scroll", true))
                } else if (moved > 12f) {
                    dragging = true
                    queue.trySend(abs(c.position.x, c.position.y))
                }
            }
        }
        val held = android.os.SystemClock.uptimeMillis() - start
        when {
            maxPointers >= 3 -> {
                // Three fingers: switch workspace (left/right) or overview (up/down).
                val dir = when {
                    abs(swipeX) >= abs(swipeY) && abs(swipeX) > swipeMin -> if (swipeX < 0) "left" else "right"
                    abs(swipeY) > swipeMin -> if (swipeY < 0) "up" else "down"
                    else -> null
                }
                dir?.let { queue.trySend(JSONObject().put("konnektaSwipe", it)) }
            }
            dragging || maxPointers >= 2 -> {}
            moved < 12f && isLongPress(held) -> queue.trySend(abs(down.position.x, down.position.y).put("rightclick", true))
            moved < 12f -> queue.trySend(abs(down.position.x, down.position.y).put("singleclick", true))
        }
    }
}

/** A press this long is a right click. */
private fun isLongPress(heldMs: Long) = heldMs in 550..5000
