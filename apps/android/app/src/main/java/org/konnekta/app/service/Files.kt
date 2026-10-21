package org.konnekta.app.service

import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import org.json.JSONObject
import org.konnekta.app.core.Core
import java.io.File

/** Received files go to Download/Konnekta; outgoing ones are staged in the cache. */
object Files {
    const val FOLDER = "Konnekta"

    /** A name that cannot leave the target folder or hide itself. */
    fun safeName(raw: String?): String {
        val n = (raw ?: "").substringAfterLast('/').substringAfterLast('\\').filter { it >= ' ' }.trim()
        return when {
            n.isEmpty() -> "file"
            n.startsWith(".") -> "file_$n"
            else -> n.take(200)
        }
    }

    fun mimeOf(name: String): String =
        MimeTypeMap.getSingleton().getMimeTypeFromExtension(name.substringAfterLast('.', "").lowercase())
            ?: "application/octet-stream"

    /**
     * Download a payload announced by the core into Download/Konnekta.
     * Returns the uri to open, or throws.
     */
    fun receive(ctx: Context, core: Core, payloadKey: String, filename: String?): Pair<Uri, String> {
        val name = safeName(filename)
        val tmp = File(ctx.cacheDir, "incoming").apply { mkdirs() }.let { File(it, "${System.nanoTime()}-$name") }
        try {
            core.call("fetchPayload", JSONObject().put("payloadKey", payloadKey).put("path", tmp.absolutePath))
            return publish(ctx, tmp, name) to name
        } finally {
            tmp.delete()
        }
    }

    private fun publish(ctx: Context, src: File, name: String): Uri {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
            // Android 9: app folder, no storage permission needed.
            val dir = File(ctx.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS), FOLDER).apply { mkdirs() }
            val out = unique(dir, name)
            src.copyTo(out)
            return Uri.fromFile(out)
        }
        val cr = ctx.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, name)
            put(MediaStore.Downloads.MIME_TYPE, mimeOf(name))
            put(MediaStore.Downloads.RELATIVE_PATH, "${Environment.DIRECTORY_DOWNLOADS}/$FOLDER")
            put(MediaStore.Downloads.IS_PENDING, 1)
        }
        val uri = cr.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values) ?: error("cannot create $name")
        try {
            cr.openOutputStream(uri)!!.use { out -> src.inputStream().use { it.copyTo(out) } }
            cr.update(uri, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
            return uri
        } catch (e: Exception) {
            cr.delete(uri, null, null)
            throw e
        }
    }

    private fun unique(dir: File, name: String): File {
        var f = File(dir, name)
        var i = 1
        while (f.exists()) f = File(dir, "($i) $name").also { i++ }
        return f
    }

    /** Copy a picked or shared uri into the cache so the core can read it by path. */
    fun stage(ctx: Context, uri: Uri): Pair<File, Long> {
        var name = "file"
        var modified = System.currentTimeMillis()
        ctx.contentResolver.query(uri, null, null, null, null)?.use { c ->
            if (c.moveToFirst()) {
                c.getColumnIndex(OpenableColumns.DISPLAY_NAME).takeIf { it >= 0 }?.let { c.getString(it) }?.let { name = it }
                c.getColumnIndex("last_modified").takeIf { it >= 0 }?.let { c.getLong(it) }?.takeIf { it > 0 }?.let { modified = it }
            }
        }
        val dir = File(ctx.cacheDir, "outgoing/${System.nanoTime()}").apply { mkdirs() }
        val out = File(dir, safeName(name))
        ctx.contentResolver.openInputStream(uri)!!.use { input -> out.outputStream().use { input.copyTo(it) } }
        return out to modified
    }

    /** Send picked / shared uris to one computer; blocks until done. */
    fun send(ctx: Context, core: Core, deviceId: String, uris: List<Uri>) {
        for (uri in uris) {
            val (file, modified) = stage(ctx, uri)
            try {
                val body = JSONObject()
                    .put("filename", file.name)
                    .put("lastModified", modified)
                    .put("numberOfFiles", uris.size)
                core.call("sendFile", JSONObject().put("deviceId", deviceId).put("path", file.absolutePath).put("body", body))
            } finally {
                file.parentFile?.deleteRecursively()
            }
        }
    }

    fun open(ctx: Context, uri: Uri, mime: String?) {
        val i = Intent(Intent.ACTION_VIEW).setDataAndType(uri, mime ?: "*/*")
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        runCatching { ctx.startActivity(Intent.createChooser(i, null).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
    }
}
