package org.konnekta.app.ui

import android.annotation.SuppressLint
import android.content.Context
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.os.Handler
import android.os.HandlerThread
import android.util.Log
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import org.json.JSONObject
import org.konnekta.app.core.Types
import org.konnekta.app.service.CameraCaps
import org.konnekta.app.service.Streams
import org.konnekta.app.state.supports

/** Webcam choices, saved: lens, resolution, frames per second. */
object CameraSettings {
    val RESOLUTIONS = listOf(640 to 480, 1280 to 720, 1920 to 1080)
    val RATES = listOf(15, 24, 30)

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("konnekta", Context.MODE_PRIVATE)
    fun front(ctx: Context) = prefs(ctx).getBoolean("cameraFront", false)
    fun size(ctx: Context): Pair<Int, Int> = RESOLUTIONS.getOrElse(prefs(ctx).getInt("cameraRes", 1)) { RESOLUTIONS[1] }
    fun fps(ctx: Context) = prefs(ctx).getInt("cameraFps", 30)

    fun save(ctx: Context, front: Boolean, res: Int, fps: Int) =
        prefs(ctx).edit().putBoolean("cameraFront", front).putInt("cameraRes", res).putInt("cameraFps", fps).apply()

    /** Parameters of the stream request. */
    fun params(ctx: Context): JSONObject {
        val (w, h) = size(ctx)
        return JSONObject().put("facing", if (front(ctx)) "front" else "back").put("width", w).put("height", h).put("fps", fps(ctx))
    }
}

/**
 * Webcam page (long press on Webcam): live preview, lens, resolution and
 * frame rate, start / stop on the computer. While it streams, the preview
 * shows what the computer gets.
 */
@Composable
fun ColumnScope.CameraScreen(current: JSONObject?, onBack: () -> Unit) {
    val ctx = LocalContext.current
    val ui = LocalUi.current
    val id = current?.optString("id") ?: return onBack()
    val name = current.optString("name")
    val live by Streams.live.collectAsState()
    val streaming = live.containsKey("camera/$id")
    var front by remember { mutableStateOf(CameraSettings.front(ctx)) }
    var res by remember { mutableIntStateOf(CameraSettings.RESOLUTIONS.indexOf(CameraSettings.size(ctx)).coerceAtLeast(0)) }
    var fps by remember { mutableIntStateOf(CameraSettings.fps(ctx)) }
    var surface by remember { mutableStateOf<Surface?>(null) }
    // Set while the stream starts: the page lets go of the camera first.
    var starting by remember { mutableStateOf(false) }
    val start = rememberStreamStarter()
    val (w, h) = remember(front, res) {
        // The preview at a size the camera really gives (the closest to the choice).
        val (rw, rh) = CameraSettings.RESOLUTIONS[res]
        runCatching {
            val cm = ctx.getSystemService(CameraManager::class.java)
            val cid = CameraCaps.cameraId(cm, front)!!
            CameraCaps.size(cm, cid, rw, rh, SurfaceHolder::class.java).let { it.width to it.height }
        }.getOrDefault(rw to rh)
    }
    /** Save, and while streaming switch the live webcam to the new choice. */
    fun saveAll() {
        CameraSettings.save(ctx, front, res, fps)
        if (streaming) Streams.reconfigureCamera(id, CameraSettings.params(ctx))
    }

    // The page's own preview while nothing streams (the stream draws here itself).
    DisposableEffect(surface, streaming, starting, front, w, h) {
        val s = surface
        val p = if (s != null && !streaming && !starting) LocalPreview(ctx, s, front) else null
        onDispose { p?.close() }
    }
    androidx.compose.runtime.LaunchedEffect(starting, streaming) {
        // Refused or failed: back to the preview after a while.
        if (starting && !streaming) {
            kotlinx.coroutines.delay(8000)
            starting = false
        } else if (streaming) starting = false
    }
    DisposableEffect(surface) {
        Streams.cameraPreview = surface
        onDispose { if (Streams.cameraPreview == surface) Streams.cameraPreview = null }
    }

    SubHeader("Webcam", name, onBack)
    Column(Modifier.fillMaxWidth().weight(1f).verticalScroll(rememberScrollState())) {
        val portrait = LocalConfiguration.current.orientation != android.content.res.Configuration.ORIENTATION_LANDSCAPE
        Box(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(Color.Black)
                .aspectRatio(if (portrait) h.toFloat() / w else w.toFloat() / h),
            contentAlignment = Alignment.Center,
        ) {
            AndroidView(factory = { c ->
                SurfaceView(c).apply {
                    holder.addCallback(object : SurfaceHolder.Callback {
                        override fun surfaceCreated(sh: SurfaceHolder) { surface = sh.surface }
                        override fun surfaceChanged(sh: SurfaceHolder, f: Int, ww: Int, hh: Int) {}
                        override fun surfaceDestroyed(sh: SurfaceHolder) { surface = null }
                    })
                }
            }, Modifier.fillMaxSize(), update = { v -> v.holder.setFixedSize(w, h) })
        }
        Text(if (streaming) "In onda su $name: è ciò che vede il computer." else "Anteprima: il computer non riceve ancora nulla.",
            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(vertical = 8.dp))

        SectionTitle("Fotocamera")
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            FilterChip(!front, { front = false; saveAll() }, { Text("Posteriore") })
            FilterChip(front, { front = true; saveAll() }, { Text("Anteriore") })
        }
        SectionTitle("Risoluzione")
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            CameraSettings.RESOLUTIONS.forEachIndexed { i, (_, rh) ->
                FilterChip(res == i, { res = i; saveAll() }, { Text("${rh}p") })
            }
        }
        SectionTitle("Fotogrammi al secondo")
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            CameraSettings.RATES.forEach { r -> FilterChip(fps == r, { fps = r; saveAll() }, { Text("$r") }) }
        }
        if (streaming) Hint("Puoi cambiare fotocamera, risoluzione e fotogrammi anche mentre è attiva: il computer continua a ricevere. " +
            "La webcam resta accesa anche se esci dall'app o spegni lo schermo, finché non la fermi.")
        Row(Modifier.fillMaxWidth().padding(vertical = 16.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            if (streaming) OutlinedButton(onClick = { Thread { Streams.stop(id, "camera") }.start() }, Modifier.weight(1f)) {
                Text("Ferma la webcam")
            } else Button(
                onClick = {
                    val blocked = blockedReason(current, "camera", Types.CAMERA, av = true)
                    if (blocked != null) ui.toast(blocked) else {
                        starting = true
                        android.os.Handler(android.os.Looper.getMainLooper()).postDelayed(
                            { start(Streams.newRequest(id, name, "camera", CameraSettings.params(ctx))) }, 300)
                    }
                },
                modifier = Modifier.weight(1f),
            ) { Text("Usa come webcam su $name") }
        }
        Hint("Sul computer scegli «Konnekta Camera» nell'app (videochiamata, OBS…): resta disponibile finché Konnekta è aperto sul computer, " +
            "anche quando il telefono non trasmette.")
    }
}

/**
 * Camera preview on the page while it does not stream. A camera that opens
 * after the page closed is closed at once on the same thread (never left
 * open, which would block it for the webcam).
 */
private class LocalPreview(ctx: Context, private val surface: Surface, front: Boolean) {
    private val thread = HandlerThread("konnekta-preview").apply { start() }
    private val handler = Handler(thread.looper)
    private val cm = ctx.getSystemService(CameraManager::class.java)
    private var closed = false
    private var opening = false
    private var device: CameraDevice? = null

    init {
        handler.post { runCatching { open(front) }.onFailure { Log.w("KonnektaCamera", "preview", it); opening = false } }
    }

    @SuppressLint("MissingPermission")
    private fun open(front: Boolean) {
        val id = CameraCaps.cameraId(cm, front) ?: return
        opening = true
        cm.openCamera(id, object : CameraDevice.StateCallback() {
            override fun onOpened(d: CameraDevice) {
                opening = false
                if (closed) { d.close(); thread.quitSafely(); return }
                device = d
                runCatching {
                    @Suppress("DEPRECATION")
                    d.createCaptureSession(listOf(surface), object : CameraCaptureSession.StateCallback() {
                        override fun onConfigured(s: CameraCaptureSession) {
                            if (closed) return
                            runCatching {
                                s.setRepeatingRequest(d.createCaptureRequest(CameraDevice.TEMPLATE_PREVIEW).apply { addTarget(surface) }.build(), null, handler)
                            }
                        }
                        override fun onConfigureFailed(s: CameraCaptureSession) {}
                    }, handler)
                }
            }
            override fun onDisconnected(d: CameraDevice) { opening = false; d.close(); if (closed) thread.quitSafely() }
            override fun onError(d: CameraDevice, error: Int) { opening = false; d.close(); if (closed) thread.quitSafely() }
        }, handler)
    }

    fun close() = handler.post {
        closed = true
        runCatching { device?.close() }
        device = null
        if (!opening) thread.quitSafely()
    }
}
