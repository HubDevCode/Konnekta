package org.konnekta.app.service

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.os.Build
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

/**
 * New versions of the app, from the GitHub releases (as the desktop app):
 * checked at every start, shown in a popup, downloaded and handed to the
 * system installer, which always asks the user to confirm.
 */
object Updates {
    private const val TAG = "KonnektaUpdate"
    private const val API = "https://api.github.com/repos/HubDevCode/Konnekta/releases/latest"
    const val REPO = "https://github.com/HubDevCode/Konnekta"
    private const val SNOOZE_MS = 24 * 3600 * 1000L
    const val ACTION_STATUS = "org.konnekta.app.INSTALL_STATUS"

    data class Release(val version: String, val notes: String, val page: String, val apkUrl: String?, val apkName: String?)

    /** Latest check: null until done. */
    val latest = MutableStateFlow<Release?>(null)
    /** Release to show in the popup. */
    val prompt = MutableStateFlow<Release?>(null)
    /** Download progress 0..1 while it runs, or null. */
    val progress = MutableStateFlow<Float?>(null)
    /** Last error or installer message for the user. */
    val message = MutableStateFlow<String?>(null)

    fun current(ctx: Context): String =
        runCatching { ctx.packageManager.getPackageInfo(ctx.packageName, 0).versionName }.getOrNull() ?: "0"

    private fun parts(v: String): List<Int> =
        v.trimStart('v').split('.', '-', '+').map { it.toIntOrNull() }.takeWhile { it != null }.map { it!! }

    /** a > b by version numbers. */
    fun newer(a: String, b: String): Boolean {
        val (x, y) = parts(a) to parts(b)
        for (i in 0 until maxOf(x.size, y.size)) {
            val d = x.getOrElse(i) { 0 } - y.getOrElse(i) { 0 }
            if (d != 0) return d > 0
        }
        return false
    }

    /** The APK built for this phone (arm64, x86_64), else the universal one. */
    private fun pickApk(assets: org.json.JSONArray): Pair<String, String>? {
        val all = (0 until assets.length()).mapNotNull { assets.optJSONObject(it) }
            .map { it.optString("name") to it.optString("browser_download_url") }
            .filter { it.first.endsWith(".apk") && it.second.isNotEmpty() }
        val abi = Build.SUPPORTED_ABIS.firstOrNull().orEmpty()
        return all.firstOrNull { it.first.contains(abi) } ?: all.firstOrNull { it.first.contains("universal") } ?: all.firstOrNull()
    }

    /** Ask GitHub for the latest release (network, run off the main thread). */
    suspend fun check(): Release? = withContext(Dispatchers.IO) {
        val c = URL(API).openConnection() as HttpURLConnection
        c.connectTimeout = 10_000
        c.readTimeout = 15_000
        c.setRequestProperty("Accept", "application/vnd.github+json")
        c.setRequestProperty("User-Agent", "konnekta-android")
        try {
            if (c.responseCode == 404) return@withContext null // nothing published yet
            if (c.responseCode !in 200..299) error("GitHub ha risposto ${c.responseCode}")
            val o = JSONObject(c.inputStream.bufferedReader().readText())
            val apk = pickApk(o.optJSONArray("assets") ?: org.json.JSONArray())
            Release(o.optString("tag_name").trimStart('v'), o.optString("body"), o.optString("html_url"), apk?.second, apk?.first)
                .also { latest.value = it }
        } finally {
            c.disconnect()
        }
    }

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("konnekta", Context.MODE_PRIVATE)

    fun snooze(ctx: Context, r: Release) {
        prefs(ctx).edit().putLong("updateSnooze-${r.version}", System.currentTimeMillis() + SNOOZE_MS).apply()
        prompt.value = null
    }

    /** At start: a popup when a newer version is out (unless put off). */
    suspend fun checkOnStart(ctx: Context) {
        val r = runCatching { check() }.getOrNull() ?: return
        if (newer(r.version, current(ctx)) && prefs(ctx).getLong("updateSnooze-${r.version}", 0) < System.currentTimeMillis()) {
            prompt.value = r
        }
    }

    /** Download the APK and hand it to the system installer. */
    suspend fun install(ctx: Context, r: Release) = withContext(Dispatchers.IO) {
        val url = r.apkUrl ?: run {
            message.value = "Questa release non contiene l'app Android"
            return@withContext
        }
        val dir = File(ctx.cacheDir, "update").apply { mkdirs() }
        val file = File(dir, "konnekta.apk")
        try {
            progress.value = 0f
            val c = URL(url).openConnection() as HttpURLConnection
            c.connectTimeout = 15_000
            c.readTimeout = 30_000
            c.setRequestProperty("User-Agent", "konnekta-android")
            c.instanceFollowRedirects = true
            val total = c.contentLengthLong
            c.inputStream.use { inp ->
                file.outputStream().use { out ->
                    val buf = ByteArray(64 * 1024)
                    var done = 0L
                    while (true) {
                        val n = inp.read(buf)
                        if (n < 0) break
                        out.write(buf, 0, n)
                        done += n
                        if (total > 0) progress.value = done.toFloat() / total
                    }
                }
            }
            c.disconnect()
            val pi = ctx.packageManager.packageInstaller
            val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL)
            params.setAppPackageName(ctx.packageName)
            val sid = pi.createSession(params)
            pi.openSession(sid).use { s ->
                s.openWrite("konnekta.apk", 0, file.length()).use { out ->
                    file.inputStream().use { it.copyTo(out) }
                    s.fsync(out)
                }
                val status = PendingIntent.getBroadcast(
                    ctx, sid, Intent(ACTION_STATUS).setPackage(ctx.packageName),
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE,
                )
                s.commit(status.intentSender)
            }
            prompt.value = null
        } catch (e: Exception) {
            Log.w(TAG, "update failed", e)
            message.value = "Aggiornamento non riuscito: ${e.message ?: e.javaClass.simpleName}"
        } finally {
            progress.value = null
        }
    }
}

/** Answers of the system installer: the confirmation screen, or the outcome. */
class InstallReceiver : BroadcastReceiver() {
    override fun onReceive(ctx: Context, intent: Intent) {
        when (intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                @Suppress("DEPRECATION")
                val confirm: Intent? = if (Build.VERSION.SDK_INT >= 33) intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java)
                else intent.getParcelableExtra(Intent.EXTRA_INTENT)
                confirm?.let { ctx.startActivity(it.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
            }
            PackageInstaller.STATUS_SUCCESS -> Updates.message.value = "Konnekta aggiornato"
            else -> {
                val why = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE).orEmpty()
                Updates.message.value = if (why.contains("conflict", true) || why.contains("signature", true))
                    "L'aggiornamento è firmato con un'altra chiave: disinstalla questa versione e installa quella nuova dalla pagina della release"
                else "Installazione annullata${if (why.isNotEmpty()) ": $why" else ""}"
            }
        }
    }
}
