package org.konnekta.app.state

import android.content.Context
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import org.json.JSONArray
import org.json.JSONObject

/**
 * App settings (Computer › Impostazioni): layout in landscape, scroll speed,
 * order and visibility of the features on Controllo and Invia. Saved in the
 * app preferences; the grid setting stays in [Store.grid].
 */
object Settings {
    private const val PREFS = "konnekta"

    /** Where the tab bar goes when the phone is in landscape. */
    enum class NavSide { Bottom, Left, Right }

    /** The bar with the computer being controlled, in landscape. */
    enum class TopBar { Full, Compact, Hidden }

    val navLandscape = MutableStateFlow(NavSide.Left)
    val topBarLandscape = MutableStateFlow(TopBar.Compact)

    /** Touchpad scroll speed multiplier. */
    val scrollSpeed = MutableStateFlow(2f)
    const val SCROLL_MIN = 0.5f
    const val SCROLL_MAX = 5f

    /** Presentation: the volume keys change slide also with the screen off and outside the app. */
    val presenterAnywhere = MutableStateFlow(false)

    /** The computer's player in the Android media controls and lock screen. */
    val pcMediaControls = MutableStateFlow(true)

    /** Saved order per container: "control" / "send" (sections), "control/<section>" (features). */
    val order = MutableStateFlow<Map<String, List<String>>>(emptyMap())

    /** Hidden features, as "<screen>/<feature>". */
    val hidden = MutableStateFlow<Set<String>>(emptySet())

    /** Screen whose features are being rearranged ("control" / "send"), or null. */
    val editing = MutableStateFlow<String?>(null)

    fun load(ctx: Context) {
        val p = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        navLandscape.value = runCatching { NavSide.valueOf(p.getString("navLandscape", null)!!) }.getOrDefault(NavSide.Left)
        topBarLandscape.value = runCatching { TopBar.valueOf(p.getString("topBarLandscape", null)!!) }.getOrDefault(TopBar.Compact)
        scrollSpeed.value = p.getFloat("scrollSpeed", 2f).coerceIn(SCROLL_MIN, SCROLL_MAX)
        order.value = runCatching {
            val o = JSONObject(p.getString("featureOrder", "{}")!!)
            o.keys().asSequence().associateWith { k -> o.getJSONArray(k).let { a -> (0 until a.length()).map { a.getString(it) } } }
        }.getOrDefault(emptyMap())
        hidden.value = p.getStringSet("hiddenFeatures", emptySet())!!.toSet()
        presenterAnywhere.value = p.getBoolean("presenterAnywhere", false)
        pcMediaControls.value = p.getBoolean("pcMediaControls", true)
    }

    fun setPcMediaControls(ctx: Context, on: Boolean) {
        pcMediaControls.value = on
        prefs(ctx).putBoolean("pcMediaControls", on).apply()
    }

    fun setPresenterAnywhere(ctx: Context, on: Boolean) {
        presenterAnywhere.value = on
        prefs(ctx).putBoolean("presenterAnywhere", on).apply()
    }

    private fun prefs(ctx: Context) = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()

    fun setNavLandscape(ctx: Context, v: NavSide) {
        navLandscape.value = v
        prefs(ctx).putString("navLandscape", v.name).apply()
    }

    fun setTopBarLandscape(ctx: Context, v: TopBar) {
        topBarLandscape.value = v
        prefs(ctx).putString("topBarLandscape", v.name).apply()
    }

    fun setScrollSpeed(ctx: Context, v: Float) {
        scrollSpeed.value = v.coerceIn(SCROLL_MIN, SCROLL_MAX)
        prefs(ctx).putFloat("scrollSpeed", scrollSpeed.value).apply()
    }

    /** [items] sorted by the saved order of [container]; new ones keep their place at the end. */
    fun <T> arrange(container: String, items: List<T>, key: (T) -> String): List<T> {
        val saved = order.value[container] ?: return items
        val rank = saved.withIndex().associate { (i, k) -> k to i }
        return items.withIndex().sortedBy { (i, t) -> rank[key(t)] ?: (saved.size + i) }.map { it.value }
    }

    /** Move [key] one place up (-1) or down (+1) among [current] (the keys as shown). */
    fun move(ctx: Context, container: String, current: List<String>, key: String, by: Int) {
        val i = current.indexOf(key)
        val j = i + by
        if (i < 0 || j !in current.indices) return
        val l = current.toMutableList()
        l[i] = l[j].also { l[j] = l[i] }
        order.update { it + (container to l) }
        saveOrder(ctx)
    }

    fun setHidden(ctx: Context, key: String, hide: Boolean) {
        hidden.update { if (hide) it + key else it - key }
        prefs(ctx).putStringSet("hiddenFeatures", hidden.value).apply()
    }

    /** Back to the original order with every feature shown. */
    fun resetLayout(ctx: Context, screen: String) {
        order.update { m -> m.filterKeys { it != screen && !it.startsWith("$screen/") } }
        hidden.update { s -> s.filterNot { it.startsWith("$screen/") }.toSet() }
        saveOrder(ctx)
        prefs(ctx).putStringSet("hiddenFeatures", hidden.value).apply()
    }

    private fun saveOrder(ctx: Context) {
        val o = JSONObject()
        order.value.forEach { (k, v) -> o.put(k, JSONArray(v)) }
        prefs(ctx).putString("featureOrder", o.toString()).apply()
    }
}
