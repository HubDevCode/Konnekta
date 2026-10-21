package org.konnekta.app.service

import org.junit.Assert.assertEquals
import org.junit.Test

class H264Test {
    private fun hex(s: String) = ByteArray(s.length / 2) { s.substring(it * 2, it * 2 + 2).toInt(16).toByte() }

    // SPS from x264 (ffmpeg, -tune zerolatency), as the computer sends them.
    @Test
    fun fullHdWithCropping() {
        val sps = hex("0000000167f400289196401e0089f89c0440000003004000000f23c60c92")
        assertEquals(1920 to 1080, H264.spsSize(sps, 4))
    }

    @Test
    fun oddWidth() {
        val sps = hex("0000000167f400209196401581878bf0110000030001000003003c8f183248")
        assertEquals(1366 to 768, H264.spsSize(sps, 4))
    }

    @Test
    fun garbageIsNull() {
        assertEquals(null, H264.spsSize(hex("0000000167"), 4))
    }
}
