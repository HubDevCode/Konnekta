package org.konnekta.konnekta

import android.os.Handler
import android.os.Looper
import io.flutter.plugin.common.EventChannel

/** Delivers native events to Dart on the main thread. */
object Bridge {
    @Volatile var sink: EventChannel.EventSink? = null
    private val main = Handler(Looper.getMainLooper())

    fun emit(event: Map<String, Any?>) {
        main.post { sink?.success(event) }
    }

    fun error(message: String) = emit(mapOf("type" to "error", "message" to message))

    fun frame(plugin: String, kind: Int, flags: Int, ptsUs: Long, data: ByteArray) =
        emit(mapOf("type" to "frame", "plugin" to plugin, "kind" to kind, "flags" to flags, "pts" to ptsUs, "data" to data))
}
