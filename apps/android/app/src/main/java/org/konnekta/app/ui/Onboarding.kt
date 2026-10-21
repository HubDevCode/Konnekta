package org.konnekta.app.ui

import android.content.Context
import android.os.Build
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material3.Button
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.json.JSONObject
import org.konnekta.app.service.KonnektaService
import org.konnekta.app.state.Store

/** First start: name, a computer, the features to use. */
object Onboarding {
    private const val PREFS = "konnekta"

    fun needed(ctx: Context) = !ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getBoolean("onboarded", false)

    fun done(ctx: Context) = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putBoolean("onboarded", true).apply()

    fun defaultName(): String =
        (Build.MODEL ?: "Android").filter { it.isLetterOrDigit() || it == ' ' || it == '-' }.take(32).ifBlank { "Android" }
}

@Composable
fun OnboardingScreen(onDone: () -> Unit) {
    val ctx = LocalContext.current
    val ui = LocalUi.current
    val scope = rememberCoroutineScope()
    val prefs = remember { ctx.getSharedPreferences("konnekta", Context.MODE_PRIVATE) }
    var step by rememberSaveable { mutableIntStateOf(0) }
    var name by rememberSaveable { mutableStateOf(prefs.getString("deviceName", null) ?: Onboarding.defaultName()) }
    val devices by Store.devices.collectAsState()
    val nearby by Store.nearby.collectAsState()
    val paired = devices.filter { it.optBoolean("paired") }

    Column(Modifier.fillMaxSize().safeDrawingPadding().padding(20.dp)) {
        LinearProgressIndicator({ (step + 1) / 4f }, Modifier.fillMaxWidth())
        Spacer(Modifier.height(20.dp))
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            when (step) {
                0 -> {
                    Text("Benvenuto in Konnekta", style = MaterialTheme.typography.headlineMedium)
                    Text("Collega il telefono ai tuoi computer: appunti, file, notifiche, controllo remoto, audio e video. " +
                        "Tutto viaggia cifrato e solo tra dispositivi che associ tu.", color = MaterialTheme.colorScheme.onSurfaceVariant)
                    SectionTitle("Come si chiama questo telefono?")
                    OutlinedTextField(name, { name = it.take(32) }, Modifier.fillMaxWidth(), singleLine = true,
                        label = { Text("Nome mostrato ai computer") })
                }
                1 -> {
                    Text("Associa un computer", style = MaterialTheme.typography.headlineMedium)
                    Text("Avvia Konnekta sul computer, nella stessa rete Wi-Fi. Tocca il computer e controlla che mostri lo stesso codice.",
                        color = MaterialTheme.colorScheme.onSurfaceVariant)
                    paired.forEach { d ->
                        ActionCard(Icons.Filled.CheckCircle, d.optString("name"), "Associato", highlight = true) {}
                    }
                    val available = availableComputers(devices, nearby)
                    available.forEach { d -> AvailableCard(d) { pairWith(scope, ui, d) } }
                    if (available.isEmpty() && paired.isEmpty()) Hint("Cerco computer nella rete…")
                    TextButton(onClick = { scope.launch { core(ui.toast) { it.call("refresh") } } }) { Text("Cerca di nuovo") }
                    Hint("Se il computer è su un'altra rete (per esempio Tailscale), potrai aggiungerne l'indirizzo dalla scheda Computer.")
                }
                2 -> {
                    Text("Cosa vuoi usare?", style = MaterialTheme.typography.headlineMedium)
                    Text("Attiva le funzioni che ti servono: Android chiederà il permesso per ognuna. Le funzioni sensibili restano " +
                        "spente finché non le attivi tu, e puoi cambiarle quando vuoi dalla scheda Computer.",
                        color = MaterialTheme.colorScheme.onSurfaceVariant)
                    SectionTitle("Sul telefono")
                    NotificationsSetting()
                    PhoneFeatures()
                    paired.forEach { d ->
                        SectionTitle("Cosa può fare ${d.optString("name")}")
                        DevicePermissionList(d.optString("id"))
                    }
                }
                else -> {
                    Text("Tutto pronto", style = MaterialTheme.typography.headlineMedium)
                    Text(if (paired.isEmpty()) "Puoi associare un computer quando vuoi dalla scheda Computer."
                    else "${paired.joinToString { it.optString("name") }} è collegato. Trovi tutto nelle schede in basso: " +
                        "Attività, Invia, Controllo e Computer.", color = MaterialTheme.colorScheme.onSurfaceVariant)
                    ActionCard(Icons.Filled.Computer, name, "Questo telefono") {}
                }
            }
        }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            if (step > 0) TextButton(onClick = { step-- }) { Text("Indietro") }
            Spacer(Modifier.weight(1f))
            if (step == 1 && paired.isEmpty()) TextButton(onClick = { step++ }) { Text("Salta") }
            Button(
                enabled = step != 0 || name.isNotBlank(),
                onClick = {
                    when (step) {
                        0 -> {
                            val n = name.trim()
                            val changed = n != prefs.getString("deviceName", null)
                            prefs.edit().putString("deviceName", n).apply()
                            // Computers learn the name when a link opens: restart so they see the new one.
                            if (changed) {
                                scope.launch {
                                    core(ui.toast) { it.call("setConfig", JSONObject().put("deviceName", n)) }
                                    KonnektaService.restart(ctx)
                                }
                            }
                            step = 1
                        }
                        3 -> {
                            Onboarding.done(ctx)
                            onDone()
                        }
                        else -> step++
                    }
                },
            ) { Text(if (step == 3) "Inizia" else "Avanti") }
        }
    }
}
