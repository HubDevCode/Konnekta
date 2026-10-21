package org.konnekta.app.approve

import android.os.Build
import android.os.Bundle
import android.util.Base64
import androidx.activity.compose.setContent
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_STRONG
import androidx.biometric.BiometricPrompt
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.core.Types
import org.konnekta.app.ui.KonnektaTheme
import java.text.DateFormat
import java.util.Date
import java.util.concurrent.Executors
import kotlin.math.abs

/** The one request the phone is showing (approval or enrollment). */
object PendingApprovals {
    data class Request(val deviceId: String, val deviceName: String, val body: JSONObject) {
        val id: String get() = body.optString("id")
        val kind: String get() = body.optString("kind")
    }

    private val _current = MutableStateFlow<Request?>(null)
    val current: StateFlow<Request?> = _current

    fun show(r: Request) {
        _current.value = r
    }

    /** Close the request if it is still `id` (answer sent, cancel, timeout). */
    fun close(id: String) {
        if (_current.value?.id == id) _current.value = null
    }

    /** Max clock difference between computer and phone. */
    const val MAX_SKEW_SECONDS = 600L
}

class ApproveActivity : FragmentActivity() {
    private val io = Executors.newSingleThreadExecutor()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Another app drawing over this screen must not trick the user.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) window.setHideOverlayWindows(true)
        window.decorView.filterTouchesWhenObscured = true
        setContent {
            KonnektaTheme {
                val req by PendingApprovals.current.collectAsState()
                val code by EnrollCode.code.collectAsState()
                val r = req
                val c = code
                if (c != null) {
                    CodeScreen(c) {
                        EnrollCode.code.value = null
                        finish()
                    }
                } else if (r == null) {
                    finish()
                } else {
                    ApproveScreen(r, onApprove = { approve(r) }, onDeny = { answer(r, JSONObject().put("result", "denied")) })
                }
            }
        }
    }

    /** Send the answer; `after(sent)` runs on the background thread. */
    private fun answer(r: PendingApprovals.Request, body: JSONObject, after: (Boolean) -> Unit = {}) {
        PendingApprovals.close(r.id)
        io.execute {
            val sent = runCatching {
                val core = KonnektaApp.core ?: error("core not running")
                core.sendPacket(r.deviceId, Types.APPROVE, body.put("id", r.id))
            }.isSuccess
            after(sent)
        }
    }

    private fun error(r: PendingApprovals.Request, msg: String) =
        answer(r, JSONObject().put("result", "error").put("error", msg))

    private fun approve(r: PendingApprovals.Request) {
        val b = r.body
        val keys = ApproveKeys(this)
        val now = System.currentTimeMillis() / 1000
        if (abs(now - b.optLong("time")) > PendingApprovals.MAX_SKEW_SECONDS) {
            return error(r, "the phone clock and the computer clock differ too much")
        }
        try {
            when (r.kind) {
                "approve" -> {
                    val msg = ApproveMessage.approval(
                        b.optString("host"), b.optString("user"), b.optString("service"),
                        b.optString("tty"), b.optString("rhost"), b.optLong("time"), b.optString("nonce"),
                    )
                    val signer = keys.signerForApproval(r.deviceId)
                    prompt(r, signer, title = "Approva ${b.optString("service")}") { s ->
                        s.update(msg)
                        val sig = s.sign()
                        answer(r, JSONObject().put("result", "approved").put("signature", Base64.encodeToString(sig, Base64.NO_WRAP)))
                    }
                }
                "enroll" -> {
                    val (slot, signer, spki) = keys.newKey(r.deviceId)
                    val msg = ApproveMessage.enrollment(
                        b.optString("host"), b.optString("user"), ApproveKeys.sha256Hex(spki), b.optLong("time"), b.optString("nonce"),
                    )
                    prompt(r, signer, title = "Usa questo telefono per sudo", onFail = { keys.discard(r.deviceId, slot) }) { s ->
                        s.update(msg)
                        val sig = s.sign()
                        val body = JSONObject().put("result", "approved")
                            .put("publicKey", Base64.encodeToString(spki, Base64.NO_WRAP))
                            .put("signature", Base64.encodeToString(sig, Base64.NO_WRAP))
                        // The new key becomes current only once the computer has it.
                        answer(r, body) { sent ->
                            if (sent) {
                                keys.commit(r.deviceId, slot)
                                EnrollCode.show(ApproveKeys.code(spki))
                            } else {
                                keys.discard(r.deviceId, slot)
                            }
                        }
                    }
                }
                else -> error(r, "unknown request")
            }
        } catch (e: Exception) {
            error(r, e.message ?: "the phone cannot sign")
        }
    }

    /** Strong biometric only: the phone PIN can never sign. */
    private fun prompt(
        r: PendingApprovals.Request,
        signer: java.security.Signature,
        title: String,
        onFail: () -> Unit = {},
        onSigned: (java.security.Signature) -> Unit,
    ) {
        val info = BiometricPrompt.PromptInfo.Builder()
            .setTitle(title)
            .setSubtitle("${r.body.optString("user")} su ${r.body.optString("host")}")
            .setAllowedAuthenticators(BIOMETRIC_STRONG)
            .setNegativeButtonText("Annulla")
            .build()
        val bp = BiometricPrompt(this, ContextCompat.getMainExecutor(this), object : BiometricPrompt.AuthenticationCallback() {
            override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                val s = result.cryptoObject?.signature ?: return error(r, "no signature object")
                try {
                    onSigned(s)
                } catch (e: Exception) {
                    onFail()
                    error(r, e.message ?: "signing failed")
                }
            }

            override fun onAuthenticationError(code: Int, msg: CharSequence) {
                onFail()
                // Cancelled by the user: stay on the screen, they can still deny.
                if (code != BiometricPrompt.ERROR_NEGATIVE_BUTTON && code != BiometricPrompt.ERROR_USER_CANCELED) {
                    error(r, msg.toString())
                }
            }
        })
        bp.authenticate(info, BiometricPrompt.CryptoObject(signer))
    }

    override fun onDestroy() {
        io.shutdown()
        super.onDestroy()
    }
}

/** The enrollment code stays on screen until the user has typed it. */
object EnrollCode {
    val code = MutableStateFlow<String?>(null)
    fun show(c: String) {
        code.value = c
    }
}

@Composable
private fun CodeScreen(code: String, onDone: () -> Unit) {
    Surface(Modifier.fillMaxSize()) {
        Column(Modifier.padding(24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Spacer(Modifier.height(32.dp))
            Text("Scrivi questo codice nel terminale del computer", style = MaterialTheme.typography.titleLarge)
            Text(code, style = MaterialTheme.typography.displaySmall)
            Text("Scrivilo solo nel terminale dove hai lanciato `konnekta approve enroll`.", style = MaterialTheme.typography.bodySmall)
            Button(onClick = onDone) { Text("Fatto") }
        }
    }
}

@Composable
private fun ApproveScreen(r: PendingApprovals.Request, onApprove: () -> Unit, onDeny: () -> Unit) {
    val b = r.body
    var busy by remember { mutableStateOf(false) }
    Surface(Modifier.fillMaxSize()) {
        Column(Modifier.padding(24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Spacer(Modifier.height(32.dp))
            if (r.kind == "enroll") {
                Text("Usare questo telefono per approvare sudo?", style = MaterialTheme.typography.headlineSmall)
                Text("Utente ${b.optString("user")} sul computer ${b.optString("host")} (${r.deviceName}).")
                Text("Dopo l'impronta il telefono mostra un codice: scrivilo nel terminale del computer.")
            } else {
                Text("Approvare ${b.optString("service")}?", style = MaterialTheme.typography.headlineSmall)
                Text("Utente: ${b.optString("user")}")
                Text("Computer: ${b.optString("host")} (${r.deviceName})")
                if (b.optString("tty").isNotEmpty()) Text("Terminale: ${b.optString("tty")}")
                if (b.optString("rhost").isNotEmpty()) Text("Da remoto: ${b.optString("rhost")}")
                Text("Ora: ${DateFormat.getTimeInstance().format(Date(b.optLong("time") * 1000))}")
                Text(
                    "Approva solo se hai appena scritto tu il comando.",
                    style = MaterialTheme.typography.bodySmall,
                )
            }
            Spacer(Modifier.height(16.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                OutlinedButton(onClick = onDeny) { Text("Rifiuta") }
                Button(enabled = !busy, onClick = { busy = true; onApprove(); busy = false }) { Text("Approva") }
            }
        }
    }
}
