package org.konnekta.app.service

import android.Manifest
import android.content.ContentUris
import android.content.Context
import android.content.pm.PackageManager
import android.database.ContentObserver
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.util.Log
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.core.Types
import java.io.File
import java.security.MessageDigest
import java.util.concurrent.Executors

/**
 * Screenshots taken on the phone go to the computers that want them
 * (`konnekta.screenshot` with the file and its SHA-256, so the computer can
 * drop duplicates). A computer turns this on or off with {config:{autoTransfer}}.
 */
class Screenshots(private val ctx: Context) {
    companion object {
        const val TAG = "Konnekta"
        val PERMISSION = if (Build.VERSION.SDK_INT >= 33) Manifest.permission.READ_MEDIA_IMAGES else Manifest.permission.READ_EXTERNAL_STORAGE
        private const val PREFS = "konnekta"
    }

    private val io = Executors.newSingleThreadExecutor()
    private val prefs = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    private var lastId = 0L
    private val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
        override fun onChange(selfChange: Boolean) {
            io.execute { check() }
        }
    }

    private fun granted() = ctx.checkSelfPermission(PERMISSION) == PackageManager.PERMISSION_GRANTED

    fun start() {
        if (!granted()) return
        io.execute { lastId = newest()?.first ?: 0L }
        runCatching { ctx.contentResolver.registerContentObserver(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, true, observer) }
    }

    fun stop() {
        runCatching { ctx.contentResolver.unregisterContentObserver(observer) }
        io.shutdown()
    }

    /** konnekta.screenshot without a payload carries the computer's choice. */
    fun configure(deviceId: String, b: JSONObject) {
        b.optJSONObject("config")?.let { prefs.edit().putBoolean("shots-$deviceId", it.optBoolean("autoTransfer", true)).apply() }
    }

    private fun wanted(deviceId: String) = prefs.getBoolean("shots-$deviceId", true)

    /** Newest image that looks like a screenshot: (id, display name). */
    private fun newest(): Pair<Long, String>? {
        val col = if (Build.VERSION.SDK_INT >= 29) MediaStore.Images.Media.RELATIVE_PATH else MediaStore.Images.Media.DATA
        return runCatching {
            ctx.contentResolver.query(
                MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
                arrayOf(MediaStore.Images.Media._ID, MediaStore.Images.Media.DISPLAY_NAME),
                "$col LIKE ?", arrayOf("%Screenshot%"),
                "${MediaStore.Images.Media._ID} DESC",
            )?.use { c -> if (c.moveToFirst()) c.getLong(0) to c.getString(1) else null }
        }.getOrNull()
    }

    private fun check() {
        val (id, name) = newest() ?: return
        if (id <= lastId) return
        lastId = id
        val targets = connectedPaired().filter(::wanted)
        if (targets.isEmpty()) return
        // The file may still be written when MediaStore announces it.
        Thread.sleep(800)
        val uri = ContentUris.withAppendedId(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, id)
        val tmp = File(ctx.cacheDir, "outgoing/shot-$id").apply { mkdirs() }.let { File(it, Files.safeName(name)) }
        try {
            ctx.contentResolver.openInputStream(uri)!!.use { i -> tmp.outputStream().use { i.copyTo(it) } }
            val sha = MessageDigest.getInstance("SHA-256").digest(tmp.readBytes()).joinToString("") { "%02x".format(it) }
            val body = JSONObject().put("filename", tmp.name).put("sha256", sha)
            for (d in targets) runCatching {
                KonnektaApp.core?.call("sendFile", JSONObject().put("deviceId", d).put("type", Types.SCREENSHOT)
                    .put("path", tmp.absolutePath).put("body", body))
            }.onFailure { Log.w(TAG, "screenshot not sent", it) }
        } catch (e: Exception) {
            Log.w(TAG, "screenshot read failed", e)
        } finally {
            tmp.parentFile?.deleteRecursively()
        }
    }
}
