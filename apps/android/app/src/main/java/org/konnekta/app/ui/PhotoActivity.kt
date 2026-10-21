package org.konnekta.app.ui

import android.Manifest
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.FileProvider
import androidx.lifecycle.lifecycleScope
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.common.InputImage
import com.google.mlkit.vision.text.TextRecognition
import com.google.mlkit.vision.text.latin.TextRecognizerOptions
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.core.Types
import org.konnekta.app.state.Store
import java.io.File
import kotlin.coroutines.resume

/**
 * A computer asked for a photo (`konnekta.photo.request`, mode photo / scan /
 * qr). Opens the camera; a photo goes back as a file, a scan or QR code as
 * text recognised on the phone (ML Kit, offline).
 */
class PhotoActivity : ComponentActivity() {
    companion object {
        const val EXTRA_DEVICE = "device"
        const val EXTRA_MODE = "mode"
    }

    private lateinit var file: File
    private lateinit var deviceId: String
    private lateinit var mode: String
    private var answered = false

    private val take = registerForActivityResult(ActivityResultContracts.TakePicture()) { ok ->
        answered = true
        lifecycleScope.launch {
            if (ok && file.length() > 0) deliver() else answer(JSONObject().put("cancelled", true))
            file.parentFile?.deleteRecursively()
            Store.dismissKind("photo")
            getSystemService(android.app.NotificationManager::class.java).cancel(org.konnekta.app.service.KonnektaService.N_PHOTO)
            finish()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        deviceId = intent.getStringExtra(EXTRA_DEVICE) ?: return finish()
        mode = intent.getStringExtra(EXTRA_MODE) ?: "photo"
        // Recreated while the camera was open: keep waiting for the same file.
        savedInstanceState?.getString("file")?.let {
            file = File(it)
            return
        }
        val dir = File(cacheDir, "photo/${System.nanoTime()}").apply { mkdirs() }
        file = File(dir, "foto-${System.currentTimeMillis()}.jpg")
        // The app declares CAMERA (webcam), so Android requires it for the system camera too.
        if (checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) launchCamera()
        else askCamera.launch(Manifest.permission.CAMERA)
    }

    private val askCamera = registerForActivityResult(ActivityResultContracts.RequestPermission()) { ok ->
        if (ok) {
            launchCamera()
        } else {
            toast("Serve il permesso della fotocamera")
            finish()
        }
    }

    private fun launchCamera() {
        take.launch(FileProvider.getUriForFile(this, "$packageName.files", file))
    }

    override fun onDestroy() {
        // Closed without a picture (back, swiped away): the computer stops waiting.
        if (isFinishing && !answered && ::deviceId.isInitialized) {
            val id = deviceId
            Thread { runCatching { KonnektaApp.core?.sendPacket(id, Types.PHOTO, JSONObject().put("cancelled", true)) } }.start()
            Store.dismissKind("photo")
        }
        super.onDestroy()
    }

    override fun onSaveInstanceState(out: Bundle) {
        super.onSaveInstanceState(out)
        out.putString("file", file.absolutePath)
    }

    private suspend fun answer(body: JSONObject) = withContext(Dispatchers.IO) {
        runCatching { KonnektaApp.core?.sendPacket(deviceId, Types.PHOTO, body) }
    }

    private suspend fun deliver() {
        if (mode == "photo") {
            val ok = withContext(Dispatchers.IO) {
                runCatching {
                    KonnektaApp.core?.call("sendFile", JSONObject().put("deviceId", deviceId).put("type", Types.PHOTO)
                        .put("path", file.absolutePath).put("body", JSONObject().put("filename", file.name)))
                }.isSuccess
            }
            toast(if (ok) "Foto inviata a ${Store.name(deviceId)}" else "Foto non inviata")
            return
        }
        val text = recognise()
        if (text.isNullOrBlank()) {
            toast(if (mode == "qr") "Nessun codice QR trovato" else "Nessun testo trovato")
            answer(JSONObject().put("cancelled", true))
        } else {
            answer(JSONObject().put("text", text))
            toast(if (mode == "qr") "Codice inviato" else "Testo inviato")
        }
    }

    private suspend fun recognise(): String? {
        val image = runCatching { InputImage.fromFilePath(this, Uri.fromFile(file)) }.getOrNull() ?: return null
        return suspendCancellableCoroutine { cont ->
            if (mode == "qr") {
                BarcodeScanning.getClient().process(image)
                    .addOnSuccessListener { l -> cont.resume(l.firstNotNullOfOrNull { it.rawValue }) }
                    .addOnFailureListener { cont.resume(null) }
            } else {
                TextRecognition.getClient(TextRecognizerOptions.DEFAULT_OPTIONS).process(image)
                    .addOnSuccessListener { cont.resume(it.text) }
                    .addOnFailureListener { cont.resume(null) }
            }
        }
    }

    private fun toast(msg: String) = Toast.makeText(applicationContext, msg, Toast.LENGTH_SHORT).show()
}
