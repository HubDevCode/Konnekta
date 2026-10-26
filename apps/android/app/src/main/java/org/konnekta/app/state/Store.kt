package org.konnekta.app.state

import android.content.Context
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.atomic.AtomicLong

/** Something the computers sent or asked, shown in the Inbox. */
data class InboxItem(
    val id: Long,
    val deviceId: String,
    val deviceName: String,
    /** clipboard, text, link, file, ping, ring, approve, error */
    val kind: String,
    val title: String,
    val text: String = "",
    /** Link to open or content:// uri of a received file. */
    val target: String? = null,
    val mime: String? = null,
    val at: Long = System.currentTimeMillis(),
)

/**
 * State shared by the service (which receives packets) and the UI. Only
 * plain values: the core stays the source of truth for devices and trust.
 */
object Store {
    private val ids = AtomicLong()

    /** `devices` from the core, refreshed on events. */
    val devices = MutableStateFlow<List<JSONObject>>(emptyList())
    val nearby = MutableStateFlow<List<JSONObject>>(emptyList())

    /** The computer the screens act on (the scope chip). */
    private val _selected = MutableStateFlow<String?>(null)
    val selected: StateFlow<String?> = _selected

    val inbox = MutableStateFlow<List<InboxItem>>(emptyList())

    /** Per computer: merged `konnekta.media` state and player list. */
    val pcMedia = MutableStateFlow<Map<String, JSONObject>>(emptyMap())
    val pcPlayers = MutableStateFlow<Map<String, List<String>>>(emptyMap())
    /** Per computer: last `konnekta.mixer` state (outputs, inputs, apps). */
    val pcMixer = MutableStateFlow<Map<String, JSONObject>>(emptyMap())

    /** Per computer: Mobile Deck layout and live button states. */
    val deckLayout = MutableStateFlow<Map<String, JSONObject>>(emptyMap())
    val deckStates = MutableStateFlow<Map<String, JSONObject>>(emptyMap())

    /** Per computer: key -> {name, command}. */
    val commands = MutableStateFlow<Map<String, Map<String, JSONObject>>>(emptyMap())

    /** Per computer: last `konnekta.browse` answer (roots / list / search / error). */
    val browse = MutableStateFlow<Map<String, JSONObject>>(emptyMap())
    val browseRoots = MutableStateFlow<Map<String, JSONArray>>(emptyMap())

    /** Transfers in progress: id -> label. */
    val transfers = MutableStateFlow<Map<Long, String>>(emptyMap())

    /** LocalSend (compatibility on in the settings): status, devices seen, transfers by id. */
    val lsStatus = MutableStateFlow<JSONObject?>(null)
    val lsPeers = MutableStateFlow<List<JSONObject>>(emptyList())
    val lsTransfers = MutableStateFlow<Map<String, JSONObject>>(emptyMap())
    /** A LocalSend device waiting for "Accetta" / "Rifiuta". */
    val lsRequest = MutableStateFlow<JSONObject?>(null)

    /** Features shown two per row instead of one per line (setting). */
    val grid = MutableStateFlow(false)

    private const val PREFS = "konnekta"
    private const val MAX_INBOX = 100

    fun load(ctx: Context) {
        val p = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        _selected.value = p.getString("selected", null)
        grid.value = p.getBoolean("grid", false)
        Settings.load(ctx)
    }

    fun setGrid(ctx: Context, on: Boolean) {
        grid.value = on
        ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putBoolean("grid", on).apply()
    }

    fun select(ctx: Context, id: String?) {
        _selected.value = id
        ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString("selected", id).apply()
    }

    val paired: List<JSONObject> get() = devices.value.filter { it.optBoolean("paired") }

    /** The selected computer if still paired, else the first connected one. */
    fun current(): JSONObject? {
        val p = paired
        return p.firstOrNull { it.optString("id") == _selected.value }
            ?: p.firstOrNull { it.optBoolean("connected") }
            ?: p.firstOrNull()
    }

    fun name(id: String): String =
        devices.value.firstOrNull { it.optString("id") == id }?.optString("name")?.ifBlank { null } ?: "Computer"

    fun post(item: InboxItem) = inbox.update { (listOf(item) + it).take(MAX_INBOX) }

    fun item(deviceId: String, kind: String, title: String, text: String = "", target: String? = null, mime: String? = null) =
        post(InboxItem(ids.incrementAndGet(), deviceId, name(deviceId), kind, title, text, target, mime))

    /** An item from a device that is not a paired computer (LocalSend). */
    fun itemFrom(deviceName: String, kind: String, title: String, text: String = "", target: String? = null, mime: String? = null) =
        post(InboxItem(ids.incrementAndGet(), "", deviceName, kind, title, text, target, mime))

    fun dismiss(id: Long) = inbox.update { l -> l.filterNot { it.id == id } }

    fun dismissKind(kind: String) = inbox.update { l -> l.filterNot { it.kind == kind } }

    fun startTransfer(label: String): Long = ids.incrementAndGet().also { id -> transfers.update { it + (id to label) } }

    fun endTransfer(id: Long) = transfers.update { it - id }

    fun <T> put(flow: MutableStateFlow<Map<String, T>>, deviceId: String, value: T) = flow.update { it + (deviceId to value) }
}

/** The packet types the peer accepts (`incomingCapabilities`). */
fun JSONObject.supports(type: String): Boolean {
    val caps = optJSONArray("incomingCapabilities") ?: return false
    return (0 until caps.length()).any { caps.optString(it) == type }
}
