package org.konnekta.app.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.AspectRatio
import androidx.compose.material.icons.filled.Bluetooth
import androidx.compose.material.icons.filled.Bolt
import androidx.compose.material.icons.filled.GridView
import androidx.compose.material.icons.filled.MusicNote
import androidx.compose.material.icons.filled.Slideshow
import androidx.compose.material.icons.filled.SwapVert
import androidx.compose.material.icons.filled.Tune
import androidx.compose.material.icons.filled.ZoomIn
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.rememberCoroutineScope
import kotlinx.coroutines.launch
import org.json.JSONObject
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import org.konnekta.app.state.Settings
import org.konnekta.app.state.Store

/** Everything about the app itself: layout, features, touchpad, phone access. */
@Composable
fun SettingsScreen(onBack: () -> Unit) {
    val ctx = LocalContext.current
    val nav by Settings.navLandscape.collectAsState()
    val top by Settings.topBarLandscape.collectAsState()
    val speed by Settings.scrollSpeed.collectAsState()

    SubHeader("Impostazioni", null, onBack)
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
        SectionTitle("Aspetto")
        LayoutSetting()
        Choice(
            "Barra delle schede con il telefono girato",
            listOf(Settings.NavSide.Left to "A sinistra", Settings.NavSide.Right to "A destra", Settings.NavSide.Bottom to "In basso"),
            nav,
        ) { Settings.setNavLandscape(ctx, it) }
        Choice(
            "Barra del computer con il telefono girato",
            listOf(Settings.TopBar.Full to "Normale", Settings.TopBar.Compact to "Piccola", Settings.TopBar.Hidden to "Nascosta"),
            top,
        ) { Settings.setTopBarLandscape(ctx, it) }

        SectionTitle("Funzioni")
        ActionCard(Icons.Filled.Tune, "Personalizza Controllo", "Ordine delle sezioni e delle funzioni, funzioni nascoste") {
            Settings.editing.value = "control"
            onBack()
        }

        SectionTitle("Touchpad")
        Box14 {
            var v by remember(speed) { mutableFloatStateOf(speed) }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Filled.SwapVert, null, tint = MaterialTheme.colorScheme.primary)
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    Text("Velocità di scorrimento", style = MaterialTheme.typography.titleMedium)
                    Text("Con due dita sul touchpad: ×${"%.1f".format(v)}", style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            Slider(v, { v = it }, valueRange = Settings.SCROLL_MIN..Settings.SCROLL_MAX,
                onValueChangeFinished = { Settings.setScrollSpeed(ctx, v) })
        }

        SectionTitle("Desktop remoto e schermo secondario")
        val pointer by Settings.pointerMode.collectAsState()
        Choice(
            "Come muovi il mouse con il dito",
            listOf(Settings.PointerMode.Direct to "Tocca dove vuoi", Settings.PointerMode.Touchpad to "Come un touchpad"),
            pointer,
        ) { Settings.setPointerMode(ctx, it) }
        Hint(if (pointer == Settings.PointerMode.Direct) "Il puntatore va dove tocchi: tocca per cliccare, trascina per trascinare."
            else "Trascina per spostare il puntatore, tocca per cliccare, tocca due volte e trascina per selezionare.")
        val extendO by Settings.extendOrientation.collectAsState()
        Choice(
            "Schermo secondario",
            listOf(Settings.ScreenOrientation.Landscape to "Orizzontale", Settings.ScreenOrientation.Portrait to "Verticale"),
            extendO,
        ) { Settings.setExtendOrientation(ctx, it) }
        ExtendResolution(extendO == Settings.ScreenOrientation.Portrait)
        val desktopO by Settings.desktopOrientation.collectAsState()
        Choice(
            "Desktop remoto",
            listOf(
                Settings.ScreenOrientation.Landscape to "Orizzontale",
                Settings.ScreenOrientation.Portrait to "Verticale",
                Settings.ScreenOrientation.Auto to "Automatico",
            ),
            desktopO,
        ) { Settings.setDesktopOrientation(ctx, it) }
        Hint(
            (if (extendO == Settings.ScreenOrientation.Portrait) "Schermo secondario verticale: il computer crea un monitor alto e stretto, con il telefono in verticale. "
            else "Schermo secondario orizzontale: lo schermo del telefono gira e il monitor è largo. ") +
                when (desktopO) {
                    Settings.ScreenOrientation.Landscape -> "Il desktop remoto gira lo schermo in orizzontale, a tutto schermo."
                    Settings.ScreenOrientation.Portrait -> "Il desktop remoto resta in verticale."
                    Settings.ScreenOrientation.Auto -> "Il desktop remoto segue la rotazione del telefono."
                },
        )
        val desktopZoom by Settings.desktopZoom.collectAsState()
        ActionCard(
            Icons.Filled.ZoomIn, "Zoom con due dita sul desktop remoto",
            if (desktopZoom) "Allarga o stringi due dita per ingrandire lo schermo del computer"
            else "Spento: due dita servono solo a scorrere",
            trailing = { Switch(desktopZoom, { Settings.setDesktopZoom(ctx, it) }) },
        ) { Settings.setDesktopZoom(ctx, !desktopZoom) }
        val extendZoom by Settings.extendZoom.collectAsState()
        ActionCard(
            Icons.Filled.ZoomIn, "Zoom con due dita sullo schermo secondario",
            if (extendZoom) "Allarga o stringi due dita per ingrandire il monitor"
            else "Spento: due dita servono solo a scorrere",
            trailing = { Switch(extendZoom, { Settings.setExtendZoom(ctx, it) }) },
        ) { Settings.setExtendZoom(ctx, !extendZoom) }
        if (desktopZoom || extendZoom) Hint(
            "Più ingrandisci, più la vista segue da vicino il puntatore. Due dita che si spostano senza allargarsi " +
                "scorrono come sempre; il pulsante in alto a destra torna a 1×.",
        )
        val desktopAudio by Settings.desktopAudio.collectAsState()
        Choice(
            "Audio del desktop remoto",
            listOf(true to "Sul telefono", false to "No"),
            desktopAudio,
        ) { Settings.setDesktopAudio(ctx, it) }
        val extendAudio by Settings.extendAudio.collectAsState()
        Choice(
            "Audio dello schermo secondario",
            listOf(
                Settings.ScreenAudio.Duplicate to "Duplica",
                Settings.ScreenAudio.Separate to "Uscita a sé",
                Settings.ScreenAudio.None to "Nessuno",
            ),
            extendAudio,
        ) { Settings.setExtendAudio(ctx, it) }
        Hint(
            (if (desktopAudio) "Con il desktop remoto senti sul telefono l'audio del computer, in tempo reale. " else "") +
                when (extendAudio) {
                    Settings.ScreenAudio.Duplicate -> "Con lo schermo secondario il telefono suona lo stesso audio del computer."
                    Settings.ScreenAudio.Separate -> "Con lo schermo secondario il computer ha un'uscita audio in più, «Konnekta-Schermo» (solo Linux): " +
                        "le app che mandi lì suonano solo sul telefono, come un monitor con le casse."
                    Settings.ScreenAudio.None -> "Lo schermo secondario resta muto."
                } + " Serve il permesso «Audio di sistema» del telefono sul computer.",
        )

        SectionTitle("Lettore del computer")
        val pcMedia by Settings.pcMediaControls.collectAsState()
        ActionCard(
            Icons.Filled.MusicNote, "Nel centro di controllo e sulla schermata di blocco",
            if (pcMedia) "Copertina, titolo, comandi e barra di avanzamento di ciò che suona sul computer"
            else "Spento: il lettore del computer resta solo nella pagina Media",
            trailing = { Switch(pcMedia, { Settings.setPcMediaControls(ctx, it); org.konnekta.app.service.KonnektaService.pcMediaChanged(ctx) }) },
        ) {
            Settings.setPcMediaControls(ctx, !pcMedia)
            org.konnekta.app.service.KonnektaService.pcMediaChanged(ctx)
        }

        SectionTitle("Presentazione")
        val anywhere by Settings.presenterAnywhere.collectAsState()
        ActionCard(
            Icons.Filled.Slideshow, "Tasti del volume ovunque",
            if (anywhere) "Cambiano slide anche a schermo spento e fuori dall'app, finché non fermi la presentazione"
            else "Cambiano slide solo con la schermata Presentazione aperta",
            trailing = { Switch(anywhere, { Settings.setPresenterAnywhere(ctx, it) }) },
        ) { Settings.setPresenterAnywhere(ctx, !anywhere) }

        SectionTitle("Collegamento")
        val bt by Settings.bluetooth.collectAsState()
        val askBt = androidx.activity.compose.rememberLauncherForActivityResult(
            androidx.activity.result.contract.ActivityResultContracts.RequestPermission(),
        ) { org.konnekta.app.service.KonnektaService.refreshFeatures(ctx) }
        val setBt = { on: Boolean ->
            Settings.setBluetooth(ctx, on)
            if (on && !org.konnekta.app.service.BluetoothBridge.allowed(ctx)) askBt.launch(android.Manifest.permission.BLUETOOTH_CONNECT)
            else org.konnekta.app.service.KonnektaService.refreshFeatures(ctx)
        }
        ActionCard(
            Icons.Filled.Bluetooth, "Bluetooth",
            if (bt) "Il computer associato al telefono nelle impostazioni Bluetooth può collegarsi anche senza Wi-Fi né cavo"
            else "Spento: solo rete e cavo USB",
            trailing = { Switch(bt, { setBt(it) }) },
        ) { setBt(!bt) }

        LocalSendSettings()

        SectionTitle("Sul telefono")
        NotificationsSetting()
        PhoneFeatures()
        Spacer(Modifier.padding(bottom = 16.dp))
    }
}

/** Features one per line, or two per row. */
@Composable
fun LayoutSetting() {
    val ctx = LocalContext.current
    val grid by Store.grid.collectAsState()
    ActionCard(
        Icons.Filled.GridView, "Funzioni in griglia",
        if (grid) "Due per riga, più compatte" else "Una per riga, con la descrizione completa",
        trailing = { Switch(grid, { Store.setGrid(ctx, it) }) },
    ) { Store.setGrid(ctx, !grid) }
}

@Composable
private fun <T> Choice(title: String, options: List<Pair<T, String>>, selected: T, pick: (T) -> Unit) {
    Box14 {
        Text(title, style = MaterialTheme.typography.titleMedium)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.padding(top = 6.dp)) {
            options.forEach { (v, label) -> FilterChip(selected = v == selected, onClick = { pick(v) }, label = { Text(label) }) }
        }
    }
}

/**
 * Resolution of the second screen: a share of the phone's own, always in the
 * phone's shape. Lower resolution, bigger text and windows on the phone.
 */
@Composable
private fun ExtendResolution(portrait: Boolean) {
    val ctx = LocalContext.current
    val scale by Settings.extendScale.collectAsState()
    Box14 {
        var v by remember(scale) { mutableFloatStateOf(scale) }
        val (w, h) = extendSize(ctx, portrait, v)
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Filled.AspectRatio, null, tint = MaterialTheme.colorScheme.primary)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text("Risoluzione dello schermo secondario", style = MaterialTheme.typography.titleMedium)
                Text(
                    "$w × $h · " + when {
                        v >= .99f -> "come il telefono, tutto più piccolo e nitido"
                        v <= Settings.EXTEND_SCALE_MIN + .01f -> "tutto molto più grande"
                        else -> "tutto più grande"
                    },
                    style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        // Steps of 5%, from half the phone's resolution to all of it.
        Slider(v, { v = it }, valueRange = Settings.EXTEND_SCALE_MIN..1f, steps = 9,
            onValueChangeFinished = { Settings.setExtendScale(ctx, v) })
        Hint("Vale dalla prossima volta che apri lo schermo secondario.")
    }
}

/** A settings card with free content. */
@Composable
private fun Box14(content: @Composable () -> Unit) {
    Surface(
        shape = RoundedCornerShape(14.dp),
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        border = BorderStroke(1.dp, LocalStatus.current.border),
        modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp),
    ) {
        Column(Modifier.padding(14.dp)) { content() }
    }
}

/**
 * LocalSend compatibility: off by default. On, this phone appears in the
 * LocalSend apps of the same network and Invia shows their devices.
 */
@Composable
private fun LocalSendSettings() {
    val ui = LocalUi.current
    val scope = rememberCoroutineScope()
    val status by Store.lsStatus.collectAsState()
    LaunchedEffect(Unit) { refreshLocalSend() }
    val st = status ?: return
    val on = st.optBoolean("enabled")
    SectionTitle("LocalSend")
    ActionCard(
        Icons.AutoMirrored.Filled.Send, "Compatibilità con LocalSend",
        if (on) "Visibile come «${st.optString("alias")}» nelle app LocalSend della stessa rete"
        else "Spenta: scambia file anche con chi usa LocalSend, dalla pagina Invia",
        trailing = { Switch(on, { v -> scope.launch { setLocalSend(ui, JSONObject().put("enabled", v)) } }) },
    ) { scope.launch { setLocalSend(ui, JSONObject().put("enabled", !on)) } }
    if (on) {
        val quick = st.optBoolean("autoAccept")
        ActionCard(
            Icons.Filled.Bolt, "Ricezione rapida",
            if (quick) "Accetta i file dai dispositivi LocalSend senza chiedere" else "Chiede ogni volta prima di ricevere",
            trailing = { Switch(quick, { v -> scope.launch { setLocalSend(ui, JSONObject().put("autoAccept", v)) } }) },
        ) { scope.launch { setLocalSend(ui, JSONObject().put("autoAccept", !quick)) } }
        Hint("I dispositivi LocalSend non vengono associati: ogni invio a questo telefono va accettato (salvo la ricezione rapida) " +
            "e non possono usare nient'altro di Konnekta. I file ricevuti vanno in Download/Konnekta.")
    }
}
